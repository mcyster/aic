use std::error::Error;
use std::fmt::{Display, Formatter};
use std::str::FromStr;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UserPrompt(String);

impl UserPrompt {
    fn text(&self) -> &str {
        &self.0
    }
}

impl FromStr for UserPrompt {
    type Err = InvalidUserPrompt;

    fn from_str(unvalidated_text: &str) -> Result<Self, Self::Err> {
        let normalized_text = unvalidated_text.trim();

        if normalized_text.is_empty() {
            return Err(InvalidUserPrompt);
        }

        Ok(Self(normalized_text.to_owned()))
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct InvalidUserPrompt;

impl Display for InvalidUserPrompt {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "user prompt must not be empty")
    }
}

impl Error for InvalidUserPrompt {}

pub(crate) struct StubConversationService;

impl StubConversationService {
    pub(crate) fn respond_to(&self, user_prompt: UserPrompt) -> AssistantResponse {
        AssistantResponse(format!(
            "Stub assistant response to: {}",
            user_prompt.text()
        ))
    }
}

pub(crate) struct AssistantResponse(String);

impl Display for AssistantResponse {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::{InvalidUserPrompt, UserPrompt};

    #[test]
    fn user_prompt_rejects_empty_text() {
        let parsing_result = UserPrompt::from_str("   ");

        assert_eq!(parsing_result, Err(InvalidUserPrompt));
    }

    #[test]
    fn user_prompt_normalizes_surrounding_whitespace() {
        let user_prompt = UserPrompt::from_str("  explain Rust ownership  ")
            .expect("the user prompt should be valid");

        assert_eq!(user_prompt.text(), "explain Rust ownership");
    }
}
