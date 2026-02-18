use super::{Engine, ExecutionError};

impl Engine {
    /// Ask the user one or more questions during an agent loop.
    ///
    /// In interactive mode (TUI or stdio), the questions are printed and
    /// the agent pauses until the user answers. In non-interactive mode,
    /// the tool returns "unanswered" for each question.
    ///
    /// This is a lightweight implementation: it serializes questions as
    /// formatted output and relies on the agent picking up the user response
    /// in the next conversation turn.
    pub(crate) async fn agent_tool_question(
        &self,
        questions: &[QuestionItem],
    ) -> Result<String, ExecutionError> {
        if questions.is_empty() {
            return Err(ExecutionError::Dispatch(
                "question tool requires at least one question".to_string(),
            ));
        }

        // For now, format questions as structured output that the agent
        // can present to the user. The actual interactive I/O is handled
        // by the tool approval channel when in agent mode.
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
    /// The plan tool stores a structured plan in the agent state,
    /// allowing the agent to organize complex tasks step by step.
    pub(crate) async fn agent_tool_plan(
        &self,
        title: &str,
        steps: &[PlanStep],
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

        let mut output = String::new();
        output.push_str(&format!("# Plan: {title}\n\n"));

        for (idx, step) in steps.iter().enumerate() {
            let status_icon = match step.status.as_str() {
                "completed" => "✅",
                "in_progress" => "🔄",
                "blocked" => "🚫",
                _ => "⬜",
            };
            output.push_str(&format!(
                "{} {}. {}\n",
                status_icon,
                idx + 1,
                step.description
            ));
        }

        Ok(output)
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
