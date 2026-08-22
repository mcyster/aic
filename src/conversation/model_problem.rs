use std::error::Error;
use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "category", content = "detail", rename_all = "snake_case")]
pub(crate) enum ModelProblem {
    Issue(ModelIssue),
    Invocation(InvocationError),
}

impl ModelProblem {
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Issue(issue) => issue.message(),
            Self::Invocation(error) => error.message(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn retryable(&self) -> bool {
        match self {
            Self::Issue(issue) => issue.retryable(),
            Self::Invocation(error) => error.retryable(),
        }
    }

    pub(super) fn ensure_valid(&self) -> Result<(), InvalidModelProblem> {
        match self {
            Self::Issue(issue) => issue.ensure_valid(),
            Self::Invocation(error) => error.ensure_valid(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ModelIssue {
    Refusal {
        message: ProblemMessage,
    },
    ContextLimitExceeded {
        message: ProblemMessage,
    },
    Other {
        message: ProblemMessage,
        extensions: Map<String, Value>,
    },
}

impl ModelIssue {
    pub(crate) fn try_refusal(message: String) -> Result<Self, InvalidModelProblem> {
        Ok(Self::Refusal {
            message: ProblemMessage::try_new(message)?,
        })
    }

    pub(crate) fn try_context_limit_exceeded(message: String) -> Result<Self, InvalidModelProblem> {
        Ok(Self::ContextLimitExceeded {
            message: ProblemMessage::try_new(message)?,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn try_other(
        message: String,
        extensions: Map<String, Value>,
    ) -> Result<Self, InvalidModelProblem> {
        Ok(Self::Other {
            message: ProblemMessage::try_new(message)?,
            extensions,
        })
    }

    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Refusal { message }
            | Self::ContextLimitExceeded { message }
            | Self::Other { message, .. } => message.as_str(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn extensions(&self) -> Option<&Map<String, Value>> {
        match self {
            Self::Other { extensions, .. } => Some(extensions),
            Self::Refusal { .. } | Self::ContextLimitExceeded { .. } => None,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn retryable(&self) -> bool {
        false
    }

    fn ensure_valid(&self) -> Result<(), InvalidModelProblem> {
        match self {
            Self::Refusal { message }
            | Self::ContextLimitExceeded { message }
            | Self::Other { message, .. } => message.ensure_valid(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum InvocationError {
    Authentication { message: ProblemMessage },
    RateLimited { message: ProblemMessage },
    Transport { message: ProblemMessage },
    InvalidRequest { message: ProblemMessage },
    ProviderFailure { message: ProblemMessage },
    InvalidProviderResponse { message: ProblemMessage },
    StreamInterrupted { message: ProblemMessage },
}

impl InvocationError {
    pub(crate) fn try_authentication(message: String) -> Result<Self, InvalidModelProblem> {
        Ok(Self::Authentication {
            message: ProblemMessage::try_new(message)?,
        })
    }

    pub(crate) fn try_rate_limited(message: String) -> Result<Self, InvalidModelProblem> {
        Ok(Self::RateLimited {
            message: ProblemMessage::try_new(message)?,
        })
    }

    pub(crate) fn try_transport(message: String) -> Result<Self, InvalidModelProblem> {
        Ok(Self::Transport {
            message: ProblemMessage::try_new(message)?,
        })
    }

    pub(crate) fn try_invalid_request(message: String) -> Result<Self, InvalidModelProblem> {
        Ok(Self::InvalidRequest {
            message: ProblemMessage::try_new(message)?,
        })
    }

    pub(crate) fn try_provider_failure(message: String) -> Result<Self, InvalidModelProblem> {
        Ok(Self::ProviderFailure {
            message: ProblemMessage::try_new(message)?,
        })
    }

    pub(crate) fn try_invalid_provider_response(
        message: String,
    ) -> Result<Self, InvalidModelProblem> {
        Ok(Self::InvalidProviderResponse {
            message: ProblemMessage::try_new(message)?,
        })
    }

    pub(crate) fn try_stream_interrupted(message: String) -> Result<Self, InvalidModelProblem> {
        Ok(Self::StreamInterrupted {
            message: ProblemMessage::try_new(message)?,
        })
    }

    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Authentication { message }
            | Self::RateLimited { message }
            | Self::Transport { message }
            | Self::InvalidRequest { message }
            | Self::ProviderFailure { message }
            | Self::InvalidProviderResponse { message }
            | Self::StreamInterrupted { message } => message.as_str(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. }
                | Self::Transport { .. }
                | Self::ProviderFailure { .. }
                | Self::StreamInterrupted { .. }
        )
    }

    fn ensure_valid(&self) -> Result<(), InvalidModelProblem> {
        match self {
            Self::Authentication { message }
            | Self::RateLimited { message }
            | Self::Transport { message }
            | Self::InvalidRequest { message }
            | Self::ProviderFailure { message }
            | Self::InvalidProviderResponse { message }
            | Self::StreamInterrupted { message } => message.ensure_valid(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct ProblemMessage(String);

impl ProblemMessage {
    fn try_new(message: String) -> Result<Self, InvalidModelProblem> {
        let message = Self(message);
        message.ensure_valid()?;
        Ok(message)
    }

    fn as_str(&self) -> &str {
        &self.0
    }

    fn ensure_valid(&self) -> Result<(), InvalidModelProblem> {
        if self.0.trim().is_empty() {
            return Err(InvalidModelProblem::EmptyMessage);
        }
        Ok(())
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum InvalidModelProblem {
    EmptyMessage,
}

impl Display for InvalidModelProblem {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyMessage => write!(formatter, "model problem message must not be empty"),
        }
    }
}

impl Error for InvalidModelProblem {}

#[cfg(test)]
mod tests {
    use serde_json::{Map, json};

    use super::{InvocationError, ModelIssue, ModelProblem};

    #[test]
    fn model_problem_round_trips_with_a_closed_tagged_representation() {
        let problem = ModelProblem::Issue(
            ModelIssue::try_refusal("I cannot help with that.".to_owned())
                .expect("the refusal should be valid"),
        );

        let serialized = serde_json::to_value(&problem).expect("the problem should serialize");
        let deserialized: ModelProblem =
            serde_json::from_value(serialized.clone()).expect("the problem should deserialize");

        assert_eq!(
            serialized,
            json!({
                "category": "issue",
                "detail": {
                    "type": "refusal",
                    "message": "I cannot help with that."
                }
            })
        );
        assert_eq!(deserialized, problem);
        assert_eq!(problem.message(), "I cannot help with that.");
        assert!(!problem.retryable());
    }

    #[test]
    fn invocation_retryability_reflects_an_unchanged_later_attempt() {
        let rate_limited = ModelProblem::Invocation(
            InvocationError::try_rate_limited("Try again later.".to_owned())
                .expect("the rate limit should be valid"),
        );
        let invalid_response = ModelProblem::Invocation(
            InvocationError::try_invalid_provider_response("Invalid response.".to_owned())
                .expect("the invalid response should be valid"),
        );

        assert!(rate_limited.retryable());
        assert!(!invalid_response.retryable());
    }

    #[test]
    fn model_problem_messages_must_not_be_blank() {
        assert!(ModelIssue::try_refusal("  ".to_owned()).is_err());
        assert!(ModelIssue::try_context_limit_exceeded(String::new()).is_err());
        assert!(ModelIssue::try_other("\n".to_owned(), Map::new()).is_err());
        assert!(InvocationError::try_transport("\t".to_owned()).is_err());
    }
}
