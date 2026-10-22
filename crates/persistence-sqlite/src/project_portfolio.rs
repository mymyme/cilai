use super::{PersistenceError, SqliteRepositoryTransaction, database_error};
use contexthub_application::{
    AttentionCaseRecord, AttentionCaseRepository, PhasePlanRecord, ProjectOutcomeRepository,
    ProjectPortfolioItemRecord, ProjectPortfolioPhaseRecord, ProjectPortfolioQuery,
    ProjectPortfolioRepository, ProjectProgressRepository,
};
use contexthub_domain::{
    AttentionCaseStatus, AttentionCaseType, BusinessTaskStatus, PhasePlanStatus,
    ProjectPhaseStatus, WorkspaceId,
};
struct PortfolioProjectRow {
    project_id: String,
    display_name: String,
    goal: String,
    status: String,
    state_revision: i64,
    updated_at: i64,
    workspace_id: Option<String>,
    workspace_display_name: Option<String>,
}

struct PlanSummary {
    has_plan: bool,
    active_phases: Vec<ProjectPortfolioPhaseRecord>,
    next_phase_name: Option<String>,
}

fn count_to_i64(count: usize) -> Result<i64, PersistenceError> {
    i64::try_from(count).map_err(|_| configuration_error("portfolio_count_exceeds_range"))
}

fn configuration_error(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(super::PersistenceErrorKind::Configuration, reason, reason)
}

fn summarize_plan(plans: &[PhasePlanRecord]) -> PlanSummary {
    let current_plan = plans
        .iter()
        .find(|plan| plan.status == PhasePlanStatus::Active)
        .or_else(|| {
            plans
                .iter()
                .find(|plan| plan.status == PhasePlanStatus::Draft)
        });
    let Some(plan) = current_plan else {
        return PlanSummary {
            has_plan: false,
            active_phases: Vec::new(),
            next_phase_name: None,
        };
    };
    let active_phases = plan
        .phases
        .iter()
        .filter(|phase| {
            matches!(
                phase.status,
                ProjectPhaseStatus::InProgress
                    | ProjectPhaseStatus::Review
                    | ProjectPhaseStatus::Paused
            )
        })
        .map(|phase| ProjectPortfolioPhaseRecord {
            phase_id: phase.phase_id.clone(),
            name: phase.name.clone(),
            status: phase.status.as_str().to_owned(),
        })
        .collect::<Vec<_>>();
    let next_phase_name = plan
        .phases
        .iter()
        .find(|phase| phase.status == ProjectPhaseStatus::Planned)
        .map(|phase| phase.name.clone());
    PlanSummary {
        has_plan: true,
        active_phases,
        next_phase_name,
    }
}

fn attention_counts(cases: &[AttentionCaseRecord]) -> (i64, i64, i64) {
    let mut blocker_count = 0_i64;
    let mut decision_count = 0_i64;
    for case in cases
        .iter()
        .filter(|item| item.status != AttentionCaseStatus::Resolved)
    {
        match case.case_type {
            AttentionCaseType::ProjectBlocker => blocker_count += 1,
            AttentionCaseType::DecisionRequest => decision_count += 1,
            AttentionCaseType::AgentRunIncident => {}
        }
    }
    (blocker_count, decision_count, 0)
}

impl ProjectPortfolioRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn portfolio_items(
        &mut self,
        query: &ProjectPortfolioQuery,
    ) -> Result<Vec<ProjectPortfolioItemRecord>, Self::Error> {
        let projects = self.portfolio_project_rows(query)?;
        let mut items = Vec::with_capacity(projects.len());
        for row in projects {
            let workspace_ids = self.workspaces_for_project(&row.project_id)?;
            let plan_summary = summarize_plan(&self.phase_plans(&row.project_id)?);
            let tasks = self.project_tasks(&row.project_id)?;
            let task_total = count_to_i64(tasks.len())?;
            let task_completed = count_to_i64(
                tasks
                    .iter()
                    .filter(|task| task.status == BusinessTaskStatus::Completed)
                    .count(),
            )?;
            let deliverables = self.deliverables_for_project(&row.project_id)?;
            let deliverable_total = count_to_i64(deliverables.len())?;
            let deliverable_accepted = count_to_i64(
                deliverables
                    .iter()
                    .filter(|item| item.current_accepted_version_id.is_some())
                    .count(),
            )?;
            let (blocker_count, decision_count, incident_count) =
                attention_counts(&self.attention_cases(&row.project_id)?);

            items.push(ProjectPortfolioItemRecord {
                project_id: row.project_id,
                display_name: row.display_name,
                goal: row.goal,
                status: row.status,
                workspace_id: row.workspace_id.map(WorkspaceId::new),
                workspace_display_name: row.workspace_display_name,
                workspace_ids,
                state_revision: row.state_revision,
                updated_at: row.updated_at,
                has_plan: plan_summary.has_plan,
                active_phases: plan_summary.active_phases,
                next_phase_name: plan_summary.next_phase_name,
                task_total,
                task_completed,
                deliverable_total,
                deliverable_accepted,
                blocker_count,
                decision_count,
                incident_count,
            });
        }
        Ok(items)
    }
}

impl SqliteRepositoryTransaction<'_> {
    fn portfolio_project_rows(
        &self,
        _query: &ProjectPortfolioQuery,
    ) -> Result<Vec<PortfolioProjectRow>, PersistenceError> {
        let projects_sql =
            "SELECT p.project_id, p.display_name, p.goal, p.status, p.state_revision, p.updated_at, pw.workspace_id, w.display_name
             FROM projects p
             LEFT JOIN project_workspaces pw ON pw.project_id = p.project_id AND pw.role = 'primary'
             LEFT JOIN workspaces w ON w.workspace_id = pw.workspace_id
             ORDER BY p.created_at, p.project_id";
        let mut statement = self
            .transaction
            .prepare(projects_sql)
            .map_err(database_error("prepare portfolio projects"))?;
        statement
            .query_map([], |row| {
                Ok(PortfolioProjectRow {
                    project_id: row.get(0)?,
                    display_name: row.get(1)?,
                    goal: row.get(2)?,
                    status: row.get(3)?,
                    state_revision: row.get(4)?,
                    updated_at: row.get(5)?,
                    workspace_id: row.get(6)?,
                    workspace_display_name: row.get(7)?,
                })
            })
            .map_err(database_error("query portfolio projects"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read portfolio projects"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contexthub_application::UnitOfWork;
    use std::fs;

    #[test]
    fn portfolio_counts_phase_free_tasks_and_all_project_workspaces() {
        let root = std::env::temp_dir().join(format!(
            "ch-project-portfolio-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let mut persistence =
            crate::SqlitePersistence::open(&root.join("contexthub.db"), "0.1.0").unwrap();
        persistence
            .connection
            .execute_batch(
                "INSERT INTO workspaces(workspace_id,display_name,local_root,local_status,state_revision,created_at,updated_at)
                 VALUES ('workspace-primary','主工作区','/tmp/portfolio-primary','ready',0,1,1),
                        ('workspace-supporting','辅助工作区','/tmp/portfolio-supporting','ready',0,1,1);
                 INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
                 VALUES ('project-portfolio','项目','目标','active',0,'op-project-portfolio','fp-project-portfolio',1,1);
                 INSERT INTO project_workspaces(project_id,workspace_id,role,created_at)
                 VALUES ('project-portfolio','workspace-primary','primary',1),
                        ('project-portfolio','workspace-supporting','supporting',2);
                 INSERT INTO business_tasks(business_task_id,project_id,phase_id,title,objective,input_context,expected_output,completion_criteria,assignee_type,assignee_label,priority,due_at,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
                 VALUES ('task-current','project-portfolio',NULL,'当前任务','目标','','产出','标准','collaboration','AI 对话协作','medium',NULL,'review',0,'op-task-current','fp-task-current',2,2),
                        ('task-completed','project-portfolio',NULL,'已完成任务','目标','','产出','标准','collaboration','AI 对话协作','medium',NULL,'completed',0,'op-task-completed','fp-task-completed',3,3);
                 INSERT INTO attention_cases(attention_case_id,case_type,project_id,title,detail,status,responsibility_type,responsibility_label,reporter_label,priority,agent_run_id,agent_label,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
                 VALUES ('old-incident','agent_run_incident','project-portfolio','旧运行异常','保留记录','open','user','用户','系统','medium','old-run','AI 工作归类',0,'op-old-incident','fp-old-incident',3,3),
                        ('real-blocker','project_blocker','project-portfolio','真实阻塞','等待材料','open','user','用户','系统','medium',NULL,NULL,0,'op-real-blocker','fp-real-blocker',3,3);",
            )
            .unwrap();

        let items = persistence
            .transact(|repositories| {
                repositories.portfolio_items(&ProjectPortfolioQuery {
                    workspace_ids: vec![
                        WorkspaceId::new("workspace-primary"),
                        WorkspaceId::new("workspace-supporting"),
                    ],
                })
            })
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].workspace_ids.len(), 2);
        assert_eq!(items[0].task_total, 2);
        assert_eq!(items[0].task_completed, 1);
        assert!(!items[0].has_plan);
        assert_eq!(items[0].incident_count, 0);
        assert_eq!(items[0].blocker_count, 1);
        let cases = persistence
            .transact(|repositories| repositories.attention_cases("project-portfolio"))
            .unwrap();
        assert_eq!(cases.len(), 2);
        drop(persistence);
        fs::remove_dir_all(root).unwrap();
    }
}
