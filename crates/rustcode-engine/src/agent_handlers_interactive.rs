use super::{AgentState, Engine, ExecutionError};
use rustcode_core::event::{PlanStepEvent, TodoItemEvent};

impl Engine {
    /// Invoke a named skill and return its content to the agent.
    ///
    /// The agent can call this tool to activate a skill's instructions
    /// explicitly, receiving the skill's full markdown content as output.
    ///
    /// # Errors
    /// Returns `ExecutionError::Dispatch` if the skill name is empty, not found,
    /// or disabled.
    pub(crate) async fn agent_tool_skill(&self, name: &str) -> Result<String, ExecutionError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ExecutionError::Dispatch(
                "skill tool requires a non-empty name".to_string(),
            ));
        }

        let skill = self.skills.get(name).ok_or_else(|| {
            let available: Vec<&str> = self.skills.all().iter().map(|s| s.name()).collect();
            if available.is_empty() {
                ExecutionError::Dispatch(format!("skill '{name}' not found; no skills are loaded"))
            } else {
                ExecutionError::Dispatch(format!(
                    "skill '{name}' not found; available: {}",
                    available.join(", ")
                ))
            }
        })?;

        if !skill.metadata.enabled {
            return Err(ExecutionError::Dispatch(format!(
                "skill '{name}' is disabled"
            )));
        }

        let mut output = String::new();
        output.push_str(&format!("# Skill: {}\n", skill.name()));
        output.push_str(&format!("Description: {}\n\n", skill.description()));
        output.push_str(&skill.content);
        Ok(output)
    }

    /// Ask the user one or more questions during an agent loop.
    pub(crate) async fn agent_tool_question(
        &self,
        questions: &[QuestionItem],
    ) -> Result<String, ExecutionError> {
        if questions.is_empty() {
            return Err(ExecutionError::Dispatch(
                "question tool requires at least one question".to_string(),
            ));
        }

        let mut output = String::new();
        output.push_str(&format!(
            "Please answer the following {} question(s):\n\n",
            questions.len()
        ));

        for (idx, q) in questions.iter().enumerate() {
            output.push_str(&format!("{}. {}\n", idx + 1, q.question));
            if !q.options.is_empty() {
                output.push_str("   Options: ");
                output.push_str(&q.options.join(", "));
                output.push('\n');
            }
            if let Some(default) = &q.default {
                output.push_str(&format!("   Default: {default}\n"));
            }
            output.push('\n');
        }

        output.push_str(
            "The agent should present these questions to the user and \
             wait for their response before continuing.",
        );

        Ok(output)
    }

    /// Create or update a plan during agent execution.
    ///
    /// Stores the plan persistently in `AgentState`. The caller
    /// (`execute_tool_calls`) emits the `PlanUpdate` event.
    pub(crate) async fn agent_tool_plan(
        &self,
        title: &str,
        steps: &[PlanStep],
        state: &mut AgentState,
    ) -> Result<String, ExecutionError> {
        if title.trim().is_empty() {
            return Err(ExecutionError::Dispatch(
                "plan tool requires a non-empty title".to_string(),
            ));
        }
        if steps.is_empty() {
            return Err(ExecutionError::Dispatch(
                "plan tool requires at least one step".to_string(),
            ));
        }

        let event_steps: Vec<PlanStepEvent> = steps
            .iter()
            .map(|s| PlanStepEvent {
                description: s.description.clone(),
                status: s.status.clone(),
            })
            .collect();

        state.plan_title = Some(title.to_string());
        state.plan_steps = event_steps;

        let completed = steps.iter().filter(|s| s.status == "completed").count();
        let in_progress = steps.iter().filter(|s| s.status == "in_progress").count();
        let total = steps.len();
        Ok(format!(
            "Plan \"{title}\" updated: {total} steps ({completed} completed, {in_progress} in progress)"
        ))
    }

    /// Update the agent's working todo list.
    ///
    /// Stores todos persistently in `AgentState`. The caller
    /// (`execute_tool_calls`) emits the `TodoUpdate` event.
    pub(crate) async fn agent_tool_todowrite(
        &self,
        todos: Vec<TodoItemEvent>,
        state: &mut AgentState,
    ) -> Result<String, ExecutionError> {
        state.todos = todos.clone();

        let completed = todos.iter().filter(|t| t.status == "completed").count();
        let pending = todos.iter().filter(|t| t.status == "pending").count();
        let in_progress = todos.iter().filter(|t| t.status == "in_progress").count();
        let total = todos.len();
        Ok(format!(
            "Todo list updated: {total} items ({completed} completed, {in_progress} in progress, {pending} pending)"
        ))
    }
}

/// A single question item for the question tool.
pub struct QuestionItem {
    pub question: String,
    pub options: Vec<String>,
    pub default: Option<String>,
}

/// A single step in a plan.
pub struct PlanStep {
    pub description: String,
    pub status: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn question_item_construction() {
        let q = QuestionItem {
            question: "What color?".to_string(),
            options: vec!["red".to_string(), "blue".to_string()],
            default: Some("blue".to_string()),
        };
        assert_eq!(q.question, "What color?");
        assert_eq!(q.options.len(), 2);
        assert_eq!(q.default.as_deref(), Some("blue"));
    }

    #[test]
    fn question_item_no_options() {
        let q = QuestionItem {
            question: "Explain your reasoning".to_string(),
            options: vec![],
            default: None,
        };
        assert!(q.options.is_empty());
        assert!(q.default.is_none());
    }

    #[test]
    fn plan_step_statuses() {
        for status in &["pending", "in_progress", "completed", "blocked"] {
            let step = PlanStep {
                description: "Test step".to_string(),
                status: status.to_string(),
            };
            assert_eq!(step.status, *status);
        }
    }
}
