use crate::{ClientFrame, Event, HelloParams, HelloResult, Label, Request, Response, ServerFrame};

pub const PROTOCOL_MIN_VERSION: u32 = 26;
pub const TERMINAL_GRANTS_V1: &str = "terminal_grants_v1";
pub const RELAY_GRANTS_V1: &str = "relay_grants_v1";
pub const ENCRYPTED_DISPLAY_V1: &str = "encrypted_display_v1";

pub(crate) fn baseline_capabilities() -> Vec<Label> {
    vec![Label::new(TERMINAL_GRANTS_V1).expect("capability is a label")]
}

pub(crate) fn has_baseline_capabilities(capabilities: &[Label]) -> bool {
    capabilities == baseline_capabilities()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolRange {
    pub min: u32,
    pub max: u32,
}

impl ProtocolRange {
    pub const CURRENT: Self = Self {
        min: PROTOCOL_MIN_VERSION,
        max: crate::PROTOCOL_VERSION,
    };

    pub fn highest_common(self, other: Self) -> Option<u32> {
        let min = self.min.max(other.min);
        let max = self.max.min(other.max);
        (self.min > 0 && other.min > 0 && min <= max).then_some(max)
    }
}

impl HelloParams {
    pub fn current(app_version: Label) -> Self {
        Self {
            protocol_version: crate::PROTOCOL_VERSION,
            protocol_min_version: (PROTOCOL_MIN_VERSION != crate::PROTOCOL_VERSION)
                .then_some(PROTOCOL_MIN_VERSION),
            capabilities: [TERMINAL_GRANTS_V1, ENCRYPTED_DISPLAY_V1, RELAY_GRANTS_V1]
                .into_iter()
                .map(|s| Label::new(s).expect("capability is a label"))
                .collect(),
            app_version,
        }
    }

    pub fn range(&self) -> ProtocolRange {
        ProtocolRange {
            min: self.protocol_min_version.unwrap_or(self.protocol_version),
            max: self.protocol_version,
        }
    }

    pub fn is_valid(&self) -> bool {
        let range = self.range();
        range.min > 0
            && range.min <= range.max
            && self.capabilities.len() <= 16
            && self
                .capabilities
                .iter()
                .enumerate()
                .all(|(index, cap)| !self.capabilities[..index].contains(cap))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiatedProtocol {
    version: u32,
    terminal_grants: bool,
    encrypted_display: bool,
    relay_grants: bool,
}

impl NegotiatedProtocol {
    pub fn negotiate(hello: &HelloParams) -> Option<Self> {
        if !hello.is_valid() {
            return None;
        }
        let version = ProtocolRange::CURRENT.highest_common(hello.range())?;
        Some(Self {
            version,
            relay_grants: hello
                .capabilities
                .iter()
                .any(|c| c.as_str() == RELAY_GRANTS_V1)
                && hello
                    .capabilities
                    .iter()
                    .any(|c| c.as_str() == ENCRYPTED_DISPLAY_V1),
            encrypted_display: hello
                .capabilities
                .iter()
                .any(|c| c.as_str() == ENCRYPTED_DISPLAY_V1),
            terminal_grants: hello
                .capabilities
                .iter()
                .any(|c| c.as_str() == TERMINAL_GRANTS_V1),
        })
    }

    pub fn from_result(hello: &HelloResult) -> Option<Self> {
        let range = ProtocolRange {
            min: hello.protocol_min_version,
            max: hello.protocol_max_version,
        };
        let version = ProtocolRange::CURRENT.highest_common(range)?;
        let relay = hello
            .capabilities
            .iter()
            .any(|c| c.as_str() == RELAY_GRANTS_V1);
        let encrypted = hello
            .capabilities
            .iter()
            .any(|c| c.as_str() == ENCRYPTED_DISPLAY_V1);
        if (relay && !encrypted)
            || (hello.push_mode.is_some() && !relay)
            || hello.protocol_version != version
            || hello.capabilities.len() > 3
            || hello.capabilities.iter().enumerate().any(|(i, c)| {
                ![TERMINAL_GRANTS_V1, ENCRYPTED_DISPLAY_V1, RELAY_GRANTS_V1].contains(&c.as_str())
                    || hello.capabilities[..i].contains(c)
            })
        {
            return None;
        }
        Some(Self {
            version,
            relay_grants: hello
                .capabilities
                .iter()
                .any(|c| c.as_str() == RELAY_GRANTS_V1)
                && hello
                    .capabilities
                    .iter()
                    .any(|c| c.as_str() == ENCRYPTED_DISPLAY_V1),
            terminal_grants: hello
                .capabilities
                .iter()
                .any(|c| c.as_str() == TERMINAL_GRANTS_V1),
            encrypted_display: hello
                .capabilities
                .iter()
                .any(|c| c.as_str() == ENCRYPTED_DISPLAY_V1),
        })
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn capabilities(&self) -> Vec<Label> {
        [
            (TERMINAL_GRANTS_V1, self.terminal_grants),
            (ENCRYPTED_DISPLAY_V1, self.encrypted_display),
            (RELAY_GRANTS_V1, self.relay_grants),
        ]
        .into_iter()
        .filter(|(_, enabled)| *enabled)
        .map(|(s, _)| Label::new(s).expect("capability is a label"))
        .collect()
    }

    pub fn relay_grants(&self) -> bool {
        self.relay_grants
    }

    pub fn allows_request(&self, request: &Request) -> bool {
        if self.version != 26 {
            return false;
        }
        match request {
            Request::TerminalChallenge(_)
            | Request::TerminalGrant(_)
            | Request::TerminalWatch(_)
            | Request::TerminalRun(_)
            | Request::TerminalSendKeys(_)
            | Request::TerminalLock(_) => self.terminal_grants,
            Request::PushRegister(p) => {
                self.encrypted_display && (p.relay_grant.is_none() || self.relay_grants)
            }
            Request::PushActivityToken(p) => {
                self.encrypted_display && (p.relay_grant.is_none() || self.relay_grants)
            }
            Request::PairComplete(p) => self.terminal_grants || p.terminal_key.is_none(),
            Request::Hello(_)
            | Request::Unpair(_)
            | Request::FlockSnapshot(_)
            | Request::AgentRead(_)
            | Request::AgentHistory(_)
            | Request::PaneRead(_)
            | Request::AgentWatch(_)
            | Request::TaskOptions(_)
            | Request::AgentChanges(_)
            | Request::AgentDiff(_)
            | Request::AgentDraft(_)
            | Request::AgentPrompt(_)
            | Request::AgentSendKeys(_)
            | Request::AgentTypeText(_)
            | Request::AgentFocus(_)
            | Request::AgentScrollBottom(_)
            | Request::AgentAnswerNotice(_)
            | Request::AgentSlashDraft(_)
            | Request::AgentStar(_)
            | Request::AgentWatchPullRequest(_)
            | Request::TaskNew(_)
            | Request::TaskFolders(_)
            | Request::TaskWorktrees(_)
            | Request::TaskSessions(_)
            | Request::TaskArchive(_)
            | Request::TaskArchiveOnMerge(_)
            | Request::WorkspaceClose(_)
            | Request::PaneClose(_)
            | Request::ApprovalList(_)
            | Request::ApprovalDecide(_)
            | Request::PushActivityEnd(_)
            | Request::AttachmentBegin(_)
            | Request::AttachmentChunk(_)
            | Request::AttachmentCommit(_)
            | Request::AttachmentAbort(_) => true,
        }
    }

    pub fn client_frame(&self, mut frame: ClientFrame) -> Result<String, &'static str> {
        if let Request::PairComplete(p) = &mut frame.request
            && !self.terminal_grants
        {
            p.terminal_key = None;
        }
        if !self.allows_request(&frame.request) {
            return Err("the machine does not support this capability");
        }
        serde_json::to_string(&frame).map_err(|_| "cannot serialize client frame")
    }

    pub fn server_frame(&self, frame: &ServerFrame) -> Result<String, &'static str> {
        if self.version != 26 {
            return Err("unsupported wire codec");
        }
        let allowed = match frame {
            ServerFrame::Result { result, .. } => match result {
                Response::TerminalChallenge { .. } | Response::TerminalGranted { .. } => {
                    self.terminal_grants
                }
                Response::Hello(_)
                | Response::Paired { .. }
                | Response::Flock(_)
                | Response::Terminal(_)
                | Response::TaskOptions(_)
                | Response::GitChanges(_)
                | Response::GitDiff(_)
                | Response::History(_)
                | Response::TaskWorktrees { .. }
                | Response::TaskSessions { .. }
                | Response::TaskStarted { .. }
                | Response::TaskArchived { .. }
                | Response::TaskFolders { .. }
                | Response::Approvals { .. }
                | Response::ApprovalResolved { .. }
                | Response::AttachmentStarted { .. }
                | Response::AttachmentStored { .. }
                | Response::Draft { .. }
                | Response::Ok => true,
            },
            ServerFrame::Event { event, .. } => match event {
                Event::AgentStatus { .. }
                | Event::AgentOutput(_)
                | Event::AgentOutputPatch(_)
                | Event::ApprovalNeeded { .. }
                | Event::ApprovalResolved { .. }
                | Event::FlockChanged { .. }
                | Event::PlanUsage { .. } => true,
                Event::Unrecognized => false,
            },
            ServerFrame::Error { .. } => true,
        };
        if !allowed {
            return Err("frame requires an unnegotiated capability");
        }
        let mut frame = frame.clone();
        if let ServerFrame::Result {
            result: Response::Hello(hello),
            ..
        } = &mut frame
            && !self.relay_grants
        {
            hello.push_mode = None;
        }
        serde_json::to_string(&frame).map_err(|_| "cannot serialize server frame")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentTarget, ErrorCode, PairCompleteParams, PairingCode, TerminalId, TerminalKey};
    use serde::Deserialize;
    use serde_json::{Value, json};

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../../docs/protocol/compatibility.json")).unwrap()
    }

    fn offer(value: Value) -> HelloParams {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn the_baseline_hello_reaches_the_exact_v25_version_check() {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct V25Hello {
            protocol_version: u32,
            app_version: Label,
        }
        let encoded = serde_json::to_value(HelloParams {
            capabilities: baseline_capabilities(),
            ..HelloParams::current(Label::new("test").unwrap())
        })
        .unwrap();
        assert_eq!(encoded, fixture()["baseline_offer"]);
        let old: V25Hello = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(old.app_version.as_str(), "test");
        assert_ne!(old.protocol_version, 25);
        assert!(NegotiatedProtocol::negotiate(&offer(fixture()["v25_offer"].clone())).is_none());
        let mut extra = encoded;
        extra["protocol_min_version"] = json!(26);
        assert!(serde_json::from_value::<V25Hello>(extra).is_err());
    }

    #[test]
    fn baseline_codecs_accept_future_offers_only_when_the_baseline_is_retained() {
        for offer in fixture()["retained_offers"].as_array().unwrap() {
            let negotiated =
                NegotiatedProtocol::negotiate(&serde_json::from_value(offer.clone()).unwrap())
                    .unwrap();
            assert_eq!(negotiated.version(), 26);
            let frame = ClientFrame {
                id: 2,
                request: Request::TerminalLock(crate::Empty {}),
            };
            assert_eq!(
                serde_json::from_str::<Value>(&negotiated.client_frame(frame).unwrap()).unwrap(),
                json!({"id":2,"method":"terminal.lock","params":{}})
            );
        }
        for offer in fixture()["unsupported_offers"].as_array().unwrap() {
            assert!(
                NegotiatedProtocol::negotiate(&serde_json::from_value(offer.clone()).unwrap())
                    .is_none()
            );
        }
    }

    #[test]
    fn baseline_apps_validate_retained_daemon_results_and_selected_revisions() {
        let mut result: HelloResult =
            serde_json::from_value(fixture()["baseline_result"].clone()).unwrap();
        for maximum in [26, 27, 28] {
            result.protocol_max_version = maximum;
            assert_eq!(
                NegotiatedProtocol::from_result(&result).unwrap().version(),
                26
            );
        }
        result.protocol_version = 27;
        assert!(NegotiatedProtocol::from_result(&result).is_none());
        result.protocol_version = 26;
        result.protocol_min_version = 27;
        assert!(NegotiatedProtocol::from_result(&result).is_none());
        result.protocol_min_version = 26;
        result
            .capabilities
            .push(Label::new("unknown_future_v1").unwrap());
        assert!(NegotiatedProtocol::from_result(&result).is_none());
    }

    #[test]
    fn omission_means_the_baseline_capabilities_and_empty_means_none() {
        let mut hello = offer(fixture()["baseline_offer"].clone());
        let enabled = NegotiatedProtocol::negotiate(&hello).unwrap();
        assert_eq!(enabled.capabilities(), baseline_capabilities());
        hello.capabilities.clear();
        let encoded = serde_json::to_value(&hello).unwrap();
        assert_eq!(encoded["capabilities"], json!([]));
        let disabled = NegotiatedProtocol::negotiate(&offer(encoded)).unwrap();
        assert!(disabled.capabilities().is_empty());
        hello.capabilities = vec![Label::new("relay_grants_v1").unwrap()];
        assert!(
            NegotiatedProtocol::negotiate(&hello)
                .unwrap()
                .capabilities()
                .is_empty()
        );
    }

    #[test]
    fn display_registration_requires_the_concrete_codec_capability() {
        let request = Request::PushRegister(crate::PushRegisterParams {
            relay_grant: None,
            apns_token: crate::PushToken::new("a".repeat(64)).unwrap(),
            environment: crate::ApnsEnvironment::Sandbox,
            notification_key: crate::NotificationKey::new(
                "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE".to_owned(),
            )
            .unwrap(),
            mute_done: false,
        });
        let mut hello = HelloParams::current(Label::new("test").unwrap());
        let current = NegotiatedProtocol::negotiate(&hello).unwrap();
        assert!(current.allows_request(&request));
        let encoded = serde_json::to_value(&hello).unwrap();
        assert!(
            encoded["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!(ENCRYPTED_DISPLAY_V1))
        );
        hello.capabilities = baseline_capabilities();
        let legacy = NegotiatedProtocol::negotiate(&hello).unwrap();
        assert!(!legacy.allows_request(&request));
        assert!(legacy.client_frame(ClientFrame { id: 1, request }).is_err());
        assert_eq!(
            serde_json::to_value(&hello).unwrap(),
            fixture()["baseline_offer"]
        );
    }

    #[test]
    fn unnegotiated_pairing_fields_methods_and_response_variants_never_serialize() {
        let mut hello = HelloParams::current(Label::new("test").unwrap());
        hello.capabilities.clear();
        let negotiated = NegotiatedProtocol::negotiate(&hello).unwrap();
        let pairing = Request::PairComplete(PairCompleteParams {
            pairing_code: PairingCode::new("Zm9vYmFyYmF6cXV4cXV1dQ").unwrap(),
            device_label: Label::new("phone").unwrap(),
            terminal_key: Some(TerminalKey::new(format!("{}Q", "M".repeat(121))).unwrap()),
        });
        assert!(!negotiated.allows_request(&pairing));
        let encoded = negotiated
            .client_frame(ClientFrame {
                id: 1,
                request: pairing,
            })
            .unwrap();
        assert!(
            serde_json::from_str::<Value>(&encoded).unwrap()["params"]
                .get("terminal_key")
                .is_none()
        );
        let terminal_id = TerminalId::new("t1").unwrap();
        assert!(
            negotiated
                .client_frame(ClientFrame {
                    id: 2,
                    request: Request::TerminalChallenge(AgentTarget {
                        terminal_id: terminal_id.clone()
                    })
                })
                .is_err()
        );
        assert!(
            negotiated
                .server_frame(&ServerFrame::Result {
                    id: 2,
                    result: Response::TerminalGranted {
                        terminal_id,
                        ttl_ms: 1000
                    }
                })
                .is_err()
        );
        assert!(
            negotiated
                .server_frame(&ServerFrame::Event {
                    seq: 1,
                    event: Event::Unrecognized
                })
                .is_err()
        );
        assert!(
            negotiated
                .server_frame(&ServerFrame::Event {
                    seq: 1,
                    event: Event::FlockChanged {}
                })
                .is_ok()
        );
    }

    #[test]
    fn hello_validation_keeps_unknown_fields_duplicates_and_invalid_ranges_strict() {
        for params in [
            json!({"protocol_version":26,"protocol_min_version":27,"app_version":"test"}),
            json!({"protocol_version":0,"app_version":"test"}),
            json!({"protocol_version":26,"app_version":"test","capabilities":["terminal_grants_v1","terminal_grants_v1"]}),
            json!({"protocol_version":26,"app_version":"test","unexpected":true}),
        ] {
            let frame = json!({"id":1,"method":"hello","params":params});
            assert_eq!(
                crate::parse_client_frame(&serde_json::to_vec(&frame).unwrap())
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidParams
            );
        }
        assert!(
            ProtocolRange { min: 29, max: 28 }
                .highest_common(ProtocolRange { min: 26, max: 30 })
                .is_none()
        );
        for maximum in [28, 29, 30] {
            let app = ProtocolRange {
                min: maximum - 2,
                max: maximum,
            };
            for old in (maximum - 2)..=maximum {
                let daemon = ProtocolRange { min: old, max: old };
                assert_eq!(app.highest_common(daemon), Some(old));
                assert_eq!(daemon.highest_common(app), Some(old));
            }
            assert_eq!(
                app.highest_common(ProtocolRange {
                    min: maximum - 3,
                    max: maximum - 3
                }),
                None
            );
        }
    }
    #[test]
    fn relay_grants_require_both_concrete_capabilities_and_modes_are_gated() {
        let mut hello = HelloParams::current(Label::new("test").unwrap());
        let grant = crate::RelayGrant {
            token: crate::RelaySendToken::new(format!("v1.key.YQ.{}", "A".repeat(43))).unwrap(),
            route_id: crate::RelayRouteId::new("route_1").unwrap(),
            generation: 1,
            expires_at: 2000000000,
        };
        let request = Request::PushRegister(crate::PushRegisterParams {
            apns_token: crate::PushToken::new("a".repeat(64)).unwrap(),
            environment: crate::ApnsEnvironment::Production,
            notification_key: crate::NotificationKey::new("A".repeat(43)).unwrap(),
            mute_done: false,
            relay_grant: Some(grant.clone()),
        });
        let negotiated = NegotiatedProtocol::negotiate(&hello).unwrap();
        assert!(negotiated.allows_request(&request));
        hello.capabilities.retain(|c| c.as_str() != RELAY_GRANTS_V1);
        let previous = NegotiatedProtocol::negotiate(&hello).unwrap();
        assert!(!previous.allows_request(&request));
        let mut result: HelloResult =
            serde_json::from_value(fixture()["baseline_result"].clone()).unwrap();
        result.push_mode = Some(crate::PushMode::Relay);
        result.capabilities = negotiated.capabilities();
        assert!(
            NegotiatedProtocol::from_result(&result)
                .unwrap()
                .relay_grants()
        );
        let frame = ServerFrame::Result {
            id: 1,
            result: Response::Hello(result.clone()),
        };
        let old: Value = serde_json::from_str(&previous.server_frame(&frame).unwrap()).unwrap();
        assert!(old["result"].get("push_mode").is_none());
        result.capabilities = vec![Label::new(RELAY_GRANTS_V1).unwrap()];
        assert!(NegotiatedProtocol::from_result(&result).is_none());
        assert!(!grant.is_valid(2000000000));
        assert!(format!("{grant:?}").contains("<redacted>"));
    }
}
