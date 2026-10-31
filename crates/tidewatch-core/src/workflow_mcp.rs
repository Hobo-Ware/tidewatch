//! The `get_workflow` tool of the Tidewatch MCP server: the repo's workflow profile, so an agent
//! that lost its launch text reads the rules again instead of guessing.

use serde_json::{Value, json};

use crate::workflow_profile::Workflow;

pub const GET_TOOL: &str = "get_workflow";

#[must_use]
pub fn tool_definition() -> Value {
    json!({
        "name": GET_TOOL,
        "description": "Read how this repo works with git and GitHub: how a branch catches up, how fixes land, how to push and how a pull request opens. Call it after a context reset instead of guessing.",
        "inputSchema": {
            "type": "object",
            "properties": {},
            "additionalProperties": false,
        },
    })
}

/// The profile as JSON: each setting by name, then the rules in plain sentences.
#[must_use]
pub fn get(workflow: Workflow) -> (String, bool) {
    let profile = json!({
        "catch_up": workflow.catch_up,
        "fixes": workflow.fixes,
        "pull_requests": workflow.pull_requests,
        "rules": workflow.launch_lines(),
    });
    (serde_json::to_string_pretty(&profile).unwrap_or_default(), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_profile::{CatchUp, Fixes, PullRequests};

    #[test]
    fn the_profile_comes_back_by_setting_and_as_sentences() {
        let workflow = Workflow {
            catch_up: CatchUp::Merge,
            fixes: Fixes::NewCommits,
            pull_requests: PullRequests::Ready,
        };
        let (text, is_error) = get(workflow);
        assert!(!is_error);
        let profile: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            profile,
            json!({
                "catch_up": "merge",
                "fixes": "new-commits",
                "pull_requests": "ready",
                "rules": workflow.launch_lines(),
            })
        );
    }

    #[test]
    fn the_tool_takes_no_arguments() {
        let tool = tool_definition();
        assert_eq!(tool["name"], GET_TOOL);
        assert_eq!(tool["inputSchema"]["properties"], json!({}));
    }
}
