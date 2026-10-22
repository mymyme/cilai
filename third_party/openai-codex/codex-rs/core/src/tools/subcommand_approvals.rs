use codex_protocol::items::CommandExecutionItem;
use codex_protocol::protocol::ReviewDecision;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::MutexGuard;

#[derive(Default)]
pub(crate) struct SubcommandApprovalState {
    inner: Mutex<SubcommandApprovals>,
}

#[derive(Default)]
struct SubcommandApprovals {
    pending: HashMap<String, String>,
    declined: HashSet<String>,
    aborted: HashSet<String>,
    commands: HashMap<String, CommandExecutionItem>,
}

impl SubcommandApprovalState {
    fn lock(&self) -> MutexGuard<'_, SubcommandApprovals> {
        match self.inner.lock() {
            Ok(state) => state,
            Err(error) => panic!("subcommand approval state poisoned: {error}"),
        }
    }

    pub(crate) fn register_command(&self, item: CommandExecutionItem) {
        self.lock().commands.insert(item.id.clone(), item);
    }

    pub(crate) fn register(&self, approval_id: String, call_id: String) {
        self.lock().pending.insert(approval_id, call_id);
    }

    pub(crate) fn resolve(&self, approval_id: &str, decision: &ReviewDecision) {
        let mut state = self.lock();
        if let Some(call_id) = state.pending.remove(approval_id)
            && matches!(
                decision,
                ReviewDecision::Denied { .. } | ReviewDecision::Abort
            )
        {
            if matches!(decision, ReviewDecision::Abort) {
                state.aborted.insert(call_id.clone());
            }
            state.declined.insert(call_id);
        }
    }

    pub(crate) fn take_declined(&self, call_id: &str) -> bool {
        let mut state = self.lock();
        state.pending.retain(|_, parent| parent != call_id);
        state.declined.remove(call_id)
    }

    pub(crate) fn was_aborted(&self, call_id: &str) -> bool {
        self.lock().aborted.contains(call_id)
    }

    pub(crate) fn complete_command(&self, call_id: &str) {
        self.lock().commands.remove(call_id);
    }

    pub(crate) fn take_aborted_commands(&self) -> Vec<CommandExecutionItem> {
        let mut state = self.lock();
        let ids = state.aborted.iter().cloned().collect::<Vec<_>>();
        let mut commands = Vec::new();
        for id in ids {
            state.pending.retain(|_, parent| parent != &id);
            state.declined.remove(&id);
            if let Some(command) = state.commands.remove(&id) {
                commands.push(command);
            }
        }
        commands
    }
}
