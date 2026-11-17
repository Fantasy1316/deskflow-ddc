//! Input protocols are local to a connection. Never infer input-alt from a value
//! or change protocols during a switch/retry transaction.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum InputProtocol {
    #[default]
    Standard,
    InputAlt,
}
impl InputProtocol {
    pub fn command(self) -> &'static str {
        match self {
            Self::Standard => "input",
            Self::InputAlt => "input-alt",
        }
    }
    pub fn supported(self, platform: &str) -> bool {
        self == Self::Standard || platform == "darwin"
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProtocolChoice {
    #[default]
    Auto,
    Standard,
    InputAlt,
}
impl ProtocolChoice {
    pub fn resolve(self, detected: InputProtocol) -> InputProtocol {
        match self {
            Self::Auto => detected,
            Self::Standard => InputProtocol::Standard,
            Self::InputAlt => InputProtocol::InputAlt,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub identity: String,
    pub name: String,
    pub platform: String,
    pub protocol: InputProtocol,
    pub local_input: u16,
    pub peer_input: u16,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub protocol: InputProtocol,
    pub current_input: Option<u16>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detection {
    pub display_id: String,
    pub identity: Option<String>,
    pub readings: Vec<Reading>,
    pub recommended: Option<InputProtocol>,
    pub saved_profile: Option<Profile>,
}
impl Detection {
    pub fn new(
        display_id: String,
        identity: Option<String>,
        readings: Vec<Reading>,
        saved_profile: Option<Profile>,
    ) -> Self {
        let recommended = saved_profile
            .as_ref()
            .map(|p| p.protocol)
            .or_else(|| {
                readings
                    .iter()
                    .find(|r| r.protocol == InputProtocol::Standard && r.current_input.is_some())
                    .map(|r| r.protocol)
            })
            .or_else(|| {
                readings
                    .iter()
                    .find(|r| r.current_input.is_some())
                    .map(|r| r.protocol)
            });
        Self {
            display_id,
            identity,
            readings,
            recommended,
            saved_profile,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestReceipt {
    pub display_id: String,
    pub identity: Option<String>,
    pub protocol: InputProtocol,
    pub local_input: Option<u16>,
    pub peer_input: Option<u16>,
    pub local: bool,
    pub peer: bool,
    pub returned_local: bool,
}
impl TestReceipt {
    pub fn for_config(c: &crate::model::Config) -> Self {
        Self {
            display_id: c.display_id.clone(),
            identity: None,
            protocol: c.input_protocol(),
            local_input: c.local_input,
            peer_input: c.peer_input,
            local: false,
            peer: false,
            returned_local: false,
        }
    }
    pub fn matches(&self, c: &crate::model::Config) -> bool {
        let mut key = self.clone();
        key.local = false;
        key.peer = false;
        key.returned_local = false;
        key.identity = None;
        key == Self::for_config(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recommendations_prefer_verified_local_history_then_standard_reading() {
        let readings = vec![
            Reading {
                protocol: InputProtocol::InputAlt,
                current_input: Some(208),
                error: None,
            },
            Reading {
                protocol: InputProtocol::Standard,
                current_input: Some(15),
                error: None,
            },
        ];
        assert_eq!(
            Detection::new("id".into(), None, readings.clone(), None).recommended,
            Some(InputProtocol::Standard)
        );
        let saved = Profile {
            identity: "stable".into(),
            name: "LG".into(),
            platform: "darwin".into(),
            protocol: InputProtocol::InputAlt,
            local_input: 208,
            peer_input: 144,
        };
        assert_eq!(
            Detection::new("id".into(), None, readings, Some(saved)).recommended,
            Some(InputProtocol::InputAlt)
        );
        assert!(Detection::new("LG".into(), None, vec![], None)
            .recommended
            .is_none());
    }
    #[test]
    fn explicit_choice_and_test_receipts_do_not_follow_values_or_stale_mapping() {
        let mut c = crate::model::Config {
            local_input: Some(208),
            peer_input: Some(144),
            ..Default::default()
        };
        assert_eq!(c.input_protocol(), InputProtocol::Standard);
        let receipt = TestReceipt::for_config(&c);
        c.ddc_auto_protocol = InputProtocol::InputAlt;
        assert!(!receipt.matches(&c));
        c.ddc_protocol = ProtocolChoice::Standard;
        assert_eq!(c.input_protocol(), InputProtocol::Standard);
        assert!(receipt.matches(&c));
        c.display_id = "new monitor".into();
        assert!(!receipt.matches(&c));
        assert!(!InputProtocol::InputAlt.supported("win32"));
    }
}
