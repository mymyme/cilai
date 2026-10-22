//! 官方授权由 Runtime 执行；此处只登记各用途独立的代次及可恢复步骤。
use super::{
    MODEL_LAUNCH_PROTOCOL, ModelAuthAbortUnlaunchedParams, ModelAuthChangeState, ModelAuthIntent,
    ModelAuthLogoutPrepareParams, ModelAuthProcess, ModelAuthRecoverParams,
    ModelAuthRegisterProcessParams, ModelAuthScope, ModelAuthScopeState,
    ModelAuthStartPrepareParams, ModelAuthStep, ModelAuthStepParams, ModelAuthStepState,
    ModelConnection, ModelConnectionChange, ModelConnectionChangeState, ModelConnectionKind,
    ModelConnectionParams, ModelExecutionLease, ModelOperationParams, ModelRuntimeKind,
    PersistenceError, RootAuthChange, Transaction, Value, alive_connection, connection,
    database_error, decoded, encoded, get, list, model_active_auth_generation, next, params,
    process_alive, put, reject, valid_text,
};

fn unfinished(state: &ModelAuthChangeState) -> bool {
    !matches!(
        state,
        ModelAuthChangeState::Completed | ModelAuthChangeState::Cancelled
    )
}

pub(super) fn pending(
    tx: &Transaction<'_>,
    id: &str,
    kind: Option<ModelRuntimeKind>,
) -> Result<bool, PersistenceError> {
    Ok(list::<RootAuthChange>(tx, "authChange")?.iter().any(|op| {
        op.connection_id == id
            && unfinished(&op.state)
            && (kind.is_none()
                || op.intent == ModelAuthIntent::ChatgptLogout
                || op.runtime_kind == kind)
    }))
}

fn reserve_generation(
    tx: &Transaction<'_>,
    conn: &ModelConnection,
) -> Result<u32, PersistenceError> {
    let changes = list::<RootAuthChange>(tx, "authChange")?;
    let ordinary = list::<ModelConnectionChange>(tx, "change")?;
    next(
        changes
            .iter()
            .filter(|c| c.connection_id == conn.connection_id)
            .map(|c| c.target_generation)
            .chain(
                ordinary
                    .iter()
                    .filter(|c| c.connection.connection_id == conn.connection_id)
                    .map(|c| c.connection.auth_generation),
            )
            .chain(conn.auth_scopes.iter().map(|s| s.auth_generation))
            .chain(std::iter::once(conn.auth_generation))
            .max()
            .unwrap_or(0),
    )
}

fn check_idle(
    tx: &Transaction<'_>,
    id: &str,
    kind: Option<ModelRuntimeKind>,
) -> Result<(), PersistenceError> {
    if list::<ModelExecutionLease>(tx, "lease")?.iter().any(|l| {
        !l.released
            && l.snapshot.connection_id == id
            && kind.is_none_or(|k| l.snapshot.runtime_kind == k)
    }) {
        return Err(reject("model_connection_in_use"));
    }
    let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM ai_conversation_thread_bindings WHERE active_run_id IS NOT NULL AND json_extract(runtime_scope_json,'$.connectionId')=?1 AND (?2 IS NULL OR json_extract(runtime_scope_json,'$.runtimeKind')=?2))",params![id,kind.map(ModelRuntimeKind::as_str)],|r|r.get(0)).map_err(database_error("check OAuth native writer"))?;
    if busy {
        return Err(reject("model_connection_in_use"));
    }
    Ok(())
}

fn check_owner(op: &RootAuthChange, fence: &str) -> Result<(), PersistenceError> {
    if op.instance_fence != fence {
        return Err(reject("model_auth_instance_changed"));
    }
    if !process_alive(op.owner.pid, &op.owner.start_fingerprint)? {
        return Err(reject("model_auth_owner_unconfirmed"));
    }
    Ok(())
}

fn scope_reaped(
    op: &RootAuthChange,
    kind: ModelRuntimeKind,
    generation: u32,
) -> Result<(), PersistenceError> {
    let processes = op
        .processes
        .iter()
        .filter(|p| p.runtime_kind == kind && p.auth_generation == generation)
        .collect::<Vec<_>>();
    if processes.is_empty() {
        return Err(reject("model_auth_process_not_registered"));
    }
    for registered in processes {
        if process_alive(
            registered.process.pid,
            &registered.process.start_fingerprint,
        )? {
            return Err(reject("model_auth_process_alive"));
        }
    }
    Ok(())
}

fn prepare_steps(
    conn: &ModelConnection,
    logout: bool,
    kind: ModelRuntimeKind,
    generation: u32,
) -> Vec<ModelAuthStep> {
    if logout {
        conn.auth_scopes
            .iter()
            .filter(|s| s.state != ModelAuthScopeState::SignedOut)
            .map(|s| ModelAuthStep {
                runtime_kind: s.runtime_kind,
                auth_generation: s.auth_generation,
                state: ModelAuthStepState::Pending,
                login_id: None,
                reason: None,
            })
            .collect()
    } else {
        vec![ModelAuthStep {
            runtime_kind: kind,
            auth_generation: generation,
            state: ModelAuthStepState::Pending,
            login_id: None,
            reason: None,
        }]
    }
}

fn prepare(
    tx: &Transaction<'_>,
    params: ModelAuthStartPrepareParams,
    intent: ModelAuthIntent,
    fence: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    if !valid_text(&params.operation_id, 100) {
        return Err(reject("model_operation_id_invalid"));
    }
    let owner = params
        .owner
        .ok_or_else(|| reject("model_auth_owner_required"))?;
    if params.launch_protocol != MODEL_LAUNCH_PROTOCOL
        || !valid_text(&owner.instance_id, 200)
        || !process_alive(owner.pid, &owner.start_fingerprint)?
    {
        return Err(reject("model_auth_owner_unconfirmed"));
    }
    let logout = intent == ModelAuthIntent::ChatgptLogout;
    let metadata = matches!(
        intent,
        ModelAuthIntent::ChatgptStatus | ModelAuthIntent::ModelCatalog
    );
    if get::<ModelConnectionChange>(tx, "change", &params.operation_id)?.is_some() {
        return Err(reject("model_operation_owner_mismatch"));
    }
    if let Some(known) = get::<RootAuthChange>(tx, "authChange", &params.operation_id)? {
        if known.connection_id != params.connection_id
            || known.expected_config_revision != params.expected_config_revision
            || known.intent != intent
            || (!logout && known.runtime_kind != Some(params.runtime_kind))
        {
            return Err(reject("model_operation_conflict"));
        }
        if known.owner != owner {
            return Err(reject("model_auth_owner_mismatch"));
        }
        check_owner(&known, fence)?;
        return encoded(&known);
    }
    let mut conn = alive_connection(tx, &params.connection_id)?;
    if conn.config.kind != ModelConnectionKind::Chatgpt {
        return Err(reject("model_auth_connection_kind_invalid"));
    }
    if conn.config_revision != params.expected_config_revision {
        return Err(reject("model_connection_revision_conflict"));
    }
    if !logout && !conn.config.enabled {
        return Err(reject("model_connection_disabled"));
    }
    let kind = if logout {
        None
    } else {
        Some(params.runtime_kind)
    };
    if pending(tx, &conn.connection_id, kind)?
        || list::<ModelConnectionChange>(tx, "change")?
            .iter()
            .any(|c| {
                c.connection.connection_id == conn.connection_id
                    && c.state == ModelConnectionChangeState::Prepared
            })
    {
        return Err(reject("model_connection_auth_change_pending"));
    }
    check_idle(tx, &conn.connection_id, kind)?;
    let generation = match intent {
        ModelAuthIntent::ChatgptStatus => model_active_auth_generation(&conn, params.runtime_kind)
            .ok_or_else(|| reject("model_auth_required"))?,
        ModelAuthIntent::ModelCatalog => 0,
        _ => reserve_generation(tx, &conn)?,
    };
    let steps = prepare_steps(&conn, logout, params.runtime_kind, generation);
    let op = RootAuthChange {
        operation_id: params.operation_id,
        profile_id: conn.profile_id.clone(),
        connection_id: conn.connection_id.clone(),
        intent,
        state: ModelAuthChangeState::Prepared,
        expected_config_revision: conn.config_revision,
        target_generation: generation,
        runtime_kind: kind,
        old_scopes: conn.auth_scopes.clone(),
        steps,
        owner,
        instance_fence: fence.into(),
        launch_protocol: params.launch_protocol,
        processes: Vec::new(),
        updated_at: now,
    };
    if !metadata {
        conn.auth_generation = generation;
        conn.updated_at = now;
        put(tx, "connection", &conn.connection_id, &conn, now)?;
    }
    put(tx, "authChange", &op.operation_id, &op, now)?;
    encoded(&op)
}

fn step(
    tx: &Transaction<'_>,
    params: ModelAuthStepParams,
    fence: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut op = get::<RootAuthChange>(tx, "authChange", &params.operation_id)?
        .ok_or_else(|| reject("model_auth_operation_not_found"))?;
    check_owner(&op, fence)?;
    if matches!(
        params.state,
        ModelAuthStepState::Confirmed | ModelAuthStepState::SignedOut
    ) {
        scope_reaped(&op, params.runtime_kind, params.auth_generation)?;
    }
    if params.state == ModelAuthStepState::Started
        && !op.processes.iter().any(|p| {
            p.runtime_kind == params.runtime_kind && p.auth_generation == params.auth_generation
        })
    {
        return Err(reject("model_auth_process_not_registered"));
    }
    if params.reason.as_ref().is_some_and(|s| {
        s.len() > 128
            || s.is_empty()
            || !s
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.:-".contains(&b))
    }) || params
        .login_id
        .as_ref()
        .is_some_and(|s| !valid_text(s, 200))
    {
        return Err(reject("model_auth_step_invalid"));
    }
    let current = op
        .steps
        .iter_mut()
        .find(|s| {
            s.runtime_kind == params.runtime_kind && s.auth_generation == params.auth_generation
        })
        .ok_or_else(|| reject("model_auth_scope_mismatch"))?;
    if current.state == params.state
        && current.login_id == params.login_id
        && current.reason == params.reason
    {
        return encoded(&op);
    }
    if !unfinished(&op.state)
        || current.state == ModelAuthStepState::SignedOut
        || params.state == ModelAuthStepState::Pending
    {
        return Err(reject("model_auth_step_transition_invalid"));
    }
    if params.state == ModelAuthStepState::Started
        && op.intent == ModelAuthIntent::ChatgptLogin
        && params.login_id.is_none()
    {
        return Err(reject("model_auth_login_id_required"));
    }
    if op.intent == ModelAuthIntent::ChatgptLogout && params.state == ModelAuthStepState::Confirmed
    {
        return Err(reject("model_auth_logout_receipt_invalid"));
    }
    current.state = params.state;
    current.login_id = params.login_id;
    current.reason = params.reason;
    op.state = if current.state == ModelAuthStepState::Uncertain {
        ModelAuthChangeState::Uncertain
    } else {
        ModelAuthChangeState::InProgress
    };
    op.updated_at = now;
    put(tx, "authChange", &op.operation_id, &op, now)?;
    encoded(&op)
}

fn finish(
    tx: &Transaction<'_>,
    params: &ModelOperationParams,
    cancel: bool,
    fence: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut op = get::<RootAuthChange>(tx, "authChange", &params.operation_id)?
        .ok_or_else(|| reject("model_auth_operation_not_found"))?;
    if op.state == ModelAuthChangeState::Completed && !cancel
        || op.state == ModelAuthChangeState::Cancelled && cancel
    {
        return encoded(&op);
    }
    if !unfinished(&op.state) {
        return Err(reject("model_auth_operation_complete"));
    }
    check_owner(&op, fence)?;
    for step in &op.steps {
        scope_reaped(&op, step.runtime_kind, step.auth_generation)?;
    }
    let mut conn = alive_connection(tx, &op.connection_id)?;
    if conn.config_revision != op.expected_config_revision {
        return Err(reject("model_connection_revision_conflict"));
    }
    check_idle(tx, &conn.connection_id, op.runtime_kind)?;
    match op.intent {
        ModelAuthIntent::ChatgptLogin => {
            let step = op
                .steps
                .first()
                .ok_or_else(|| reject("model_auth_step_invalid"))?;
            if cancel {
                if step.state != ModelAuthStepState::SignedOut {
                    return Err(reject("model_auth_cancel_cleanup_required"));
                }
            } else if step.state != ModelAuthStepState::Confirmed {
                return Err(reject("model_auth_login_unconfirmed"));
            }
            let kind = op
                .runtime_kind
                .ok_or_else(|| reject("model_auth_scope_mismatch"))?;
            if !cancel {
                for scope in &mut conn.auth_scopes {
                    if scope.runtime_kind == kind && scope.state == ModelAuthScopeState::Active {
                        scope.state = ModelAuthScopeState::Retired;
                    }
                }
            }
            conn.auth_scopes.push(ModelAuthScope {
                runtime_kind: kind,
                auth_generation: op.target_generation,
                state: if cancel {
                    ModelAuthScopeState::SignedOut
                } else {
                    ModelAuthScopeState::Active
                },
                operation_id: op.operation_id.clone(),
            });
        }
        ModelAuthIntent::ChatgptLogout => {
            if cancel {
                return Err(reject("model_auth_logout_requires_completion"));
            }
            if op
                .steps
                .iter()
                .any(|s| s.state != ModelAuthStepState::SignedOut)
            {
                return Err(reject("model_auth_logout_unconfirmed"));
            }
            for scope in &mut conn.auth_scopes {
                scope.state = ModelAuthScopeState::SignedOut;
            }
        }
        ModelAuthIntent::ChatgptStatus | ModelAuthIntent::ModelCatalog => {
            let state = &op
                .steps
                .first()
                .ok_or_else(|| reject("model_auth_step_invalid"))?
                .state;
            if cancel {
                if !matches!(
                    state,
                    ModelAuthStepState::Confirmed
                        | ModelAuthStepState::Failed
                        | ModelAuthStepState::SignedOut
                ) {
                    return Err(reject("model_auth_read_unconfirmed"));
                }
            } else if *state != ModelAuthStepState::Confirmed {
                return Err(reject("model_auth_read_unconfirmed"));
            }
        }
    }
    conn.updated_at = now;
    put(tx, "connection", &conn.connection_id, &conn, now)?;
    op.state = if cancel {
        ModelAuthChangeState::Cancelled
    } else {
        ModelAuthChangeState::Completed
    };
    op.updated_at = now;
    put(tx, "authChange", &op.operation_id, &op, now)?;
    encoded(&op)
}

fn register(
    tx: &Transaction<'_>,
    params: ModelAuthRegisterProcessParams,
    fence: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut op = get::<RootAuthChange>(tx, "authChange", &params.operation_id)?
        .ok_or_else(|| reject("model_auth_operation_not_found"))?;
    check_owner(&op, fence)?;
    if !unfinished(&op.state)
        || !op.steps.iter().any(|s| {
            s.runtime_kind == params.runtime_kind
                && s.auth_generation == params.auth_generation
                && s.state != ModelAuthStepState::SignedOut
        })
    {
        return Err(reject("model_auth_scope_mismatch"));
    }
    if !process_alive(params.process.pid, &params.process.start_fingerprint)? {
        return Err(reject("model_auth_process_unconfirmed"));
    }
    let registered = ModelAuthProcess {
        runtime_kind: params.runtime_kind,
        auth_generation: params.auth_generation,
        process: params.process,
    };
    if !op.processes.contains(&registered) {
        op.processes.push(registered);
    }
    op.updated_at = now;
    put(tx, "authChange", &op.operation_id, &op, now)?;
    encoded(&op)
}

fn abort_unlaunched(
    tx: &Transaction<'_>,
    params: &ModelAuthAbortUnlaunchedParams,
    fence: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut op = get::<RootAuthChange>(tx, "authChange", &params.operation_id)?
        .ok_or_else(|| reject("model_auth_operation_not_found"))?;
    check_owner(&op, fence)?;
    if op.owner != params.owner || params.launch_protocol != MODEL_LAUNCH_PROTOCOL {
        return Err(reject("model_auth_owner_mismatch"));
    }
    if op.state == ModelAuthChangeState::Cancelled {
        return encoded(&op);
    }
    if op.intent == ModelAuthIntent::ChatgptLogout
        || op.launch_protocol != MODEL_LAUNCH_PROTOCOL
        || op.state != ModelAuthChangeState::Prepared
        || !op.processes.is_empty()
        || op.steps.len() != 1
        || op.steps[0].state != ModelAuthStepState::Pending
        || op.steps[0].login_id.is_some()
    {
        return Err(reject("model_auth_already_launched"));
    }
    if op.intent == ModelAuthIntent::ChatgptLogin {
        let mut conn = alive_connection(tx, &op.connection_id)?;
        conn.auth_scopes.push(ModelAuthScope {
            runtime_kind: op.steps[0].runtime_kind,
            auth_generation: op.target_generation,
            state: ModelAuthScopeState::SignedOut,
            operation_id: op.operation_id.clone(),
        });
        conn.updated_at = now;
        put(tx, "connection", &conn.connection_id, &conn, now)?;
        op.steps[0].state = ModelAuthStepState::SignedOut;
    } else {
        op.steps[0].state = ModelAuthStepState::Failed;
    }
    op.steps[0].reason = Some("launch_not_attempted".into());
    op.state = ModelAuthChangeState::Cancelled;
    op.updated_at = now;
    put(tx, "authChange", &op.operation_id, &op, now)?;
    encoded(&op)
}

fn recover(
    tx: &Transaction<'_>,
    params: ModelAuthRecoverParams,
    fence: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut op = get::<RootAuthChange>(tx, "authChange", &params.operation_id)?
        .ok_or_else(|| reject("model_auth_operation_not_found"))?;
    if !unfinished(&op.state) {
        return encoded(&op);
    }
    if params.launch_protocol != MODEL_LAUNCH_PROTOCOL
        || !valid_text(&params.owner.instance_id, 200)
        || !process_alive(params.owner.pid, &params.owner.start_fingerprint)?
    {
        return Err(reject("model_auth_owner_unconfirmed"));
    }
    if op.owner == params.owner {
        op.instance_fence = fence.into();
    } else {
        if process_alive(op.owner.pid, &op.owner.start_fingerprint)? {
            return Err(reject("model_auth_owner_alive"));
        }
        let unstarted_login_cleanup = params.allow_unstarted_login_cleanup
            && op.launch_protocol == MODEL_LAUNCH_PROTOCOL
            && op.intent == ModelAuthIntent::ChatgptLogin
            && op.state == ModelAuthChangeState::Prepared
            && op.steps.len() == 1
            && op.steps[0].state == ModelAuthStepState::Pending
            && op.steps[0].login_id.is_none();
        let readonly_cleanup = params.allow_unstarted_readonly_cleanup
            && matches!(
                op.intent,
                ModelAuthIntent::ChatgptStatus | ModelAuthIntent::ModelCatalog
            )
            && op.launch_protocol == MODEL_LAUNCH_PROTOCOL
            && op.state == ModelAuthChangeState::Prepared
            && op.steps.len() == 1
            && op.steps[0].state == ModelAuthStepState::Pending
            && op.steps[0].login_id.is_none();
        if op.processes.is_empty() && !unstarted_login_cleanup && !readonly_cleanup {
            return Err(reject("model_auth_ownership_unconfirmed"));
        }
        for p in &op.processes {
            if process_alive(p.process.pid, &p.process.start_fingerprint)? {
                return Err(reject("model_auth_process_alive"));
            }
        }
        op.owner = params.owner;
        op.instance_fence = fence.into();
        if op.processes.is_empty() && readonly_cleanup {
            op.state = ModelAuthChangeState::Cancelled;
        }
    }
    op.updated_at = now;
    put(tx, "authChange", &op.operation_id, &op, now)?;
    encoded(&op)
}

pub(super) fn route(
    tx: &Transaction<'_>,
    method: &str,
    params: Value,
    fence: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    match method {
        "modelAuth/startPrepare" => prepare(
            tx,
            decoded(params)?,
            ModelAuthIntent::ChatgptLogin,
            fence,
            now,
        ),
        "modelAuth/readPrepare" => prepare(
            tx,
            decoded(params)?,
            ModelAuthIntent::ChatgptStatus,
            fence,
            now,
        ),
        "modelAuth/catalogPrepare" => prepare(
            tx,
            decoded(params)?,
            ModelAuthIntent::ModelCatalog,
            fence,
            now,
        ),
        "modelAuth/logoutPrepare" => {
            let p: ModelAuthLogoutPrepareParams = decoded(params)?;
            prepare(
                tx,
                ModelAuthStartPrepareParams {
                    operation_id: p.operation_id,
                    connection_id: p.connection_id,
                    expected_config_revision: p.expected_config_revision,
                    runtime_kind: ModelRuntimeKind::Workbench,
                    owner: p.owner,
                    launch_protocol: p.launch_protocol,
                },
                ModelAuthIntent::ChatgptLogout,
                fence,
                now,
            )
        }
        "modelAuth/registerProcess" => register(tx, decoded(params)?, fence, now),
        "modelAuth/recover" => recover(tx, decoded(params)?, fence, now),
        "modelAuth/abortUnlaunched" => abort_unlaunched(tx, &decoded(params)?, fence, now),
        "modelAuth/step" => step(tx, decoded(params)?, fence, now),
        "modelAuth/finish" => finish(tx, &decoded(params)?, false, fence, now),
        "modelAuth/cancel" => finish(tx, &decoded(params)?, true, fence, now),
        "modelAuth/get" => encoded(
            &get::<RootAuthChange>(
                tx,
                "authChange",
                &decoded::<ModelOperationParams>(params)?.operation_id,
            )?
            .ok_or_else(|| reject("model_auth_operation_not_found"))?,
        ),
        "modelAuth/list" => {
            let p: ModelConnectionParams = decoded(params)?;
            connection(tx, &p.connection_id)?;
            encoded(
                &list::<RootAuthChange>(tx, "authChange")?
                    .into_iter()
                    .filter(|c| c.connection_id == p.connection_id)
                    .collect::<Vec<_>>(),
            )
        }
        _ => Err(reject("model_method_not_found")),
    }
}
