use std::time::Duration;

use futures_util::future::BoxFuture;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Request, Uri};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use hyper_util::rt::TokioExecutor;
use serde::Deserialize;
use serde_json::json;

use crate::push::{Alert, Delivery, Device, Rejection, Sender};

pub const ENDPOINT: &str = "https://collie-push.rbstp.dev/v1/send";
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE: usize = 1024;

pub struct Relay {
    client: Client<HttpsConnector<HttpConnector>, Full<Bytes>>,
    endpoint: Uri,
}

impl Relay {
    pub fn new() -> anyhow::Result<Self> {
        let connector = HttpsConnectorBuilder::new()
            .with_provider_and_platform_verifier(collie_tls::provider())?
            .https_only()
            .enable_http1()
            .enable_http2()
            .build();
        Ok(Self {
            client: Client::builder(TokioExecutor::new()).build(connector),
            endpoint: ENDPOINT.parse()?,
        })
    }

    async fn attempt(&self, device: &Device, alert: &Alert) -> Result<(), Rejection> {
        let grant = device
            .relay_grant
            .as_ref()
            .filter(|g| g.is_valid(crate::now_ms() / 1000))
            .ok_or_else(|| {
                Rejection::Other(
                    "relay grant missing or expired; open the paired app to renew".into(),
                )
            })?;
        if alert.payload.to_string().len() > 4096
            || (alert.delivery == Delivery::Alert && alert.payload.get("enc").is_none())
            || (matches!(alert.delivery, Delivery::LiveActivity { .. })
                && alert.payload["aps"]["content-state"].get("enc").is_none())
        {
            return Err(Rejection::Other(
                "encrypted display unavailable or payload too large".into(),
            ));
        }
        let (push_type, priority) = match alert.delivery {
            Delivery::Alert => ("alert", 10),
            Delivery::Background => ("background", 5),
            Delivery::LiveActivity { urgent } => ("liveactivity", if urgent { 10 } else { 5 }),
        };
        let mut body = json!({"v":1,"push_type":push_type,"priority":priority,
            "expiration":alert.expiration.unwrap_or(0),"payload":alert.payload});
        if let Some(id) = &alert.collapse_id {
            body["collapse_id"] = json!(id);
        }
        let mut authorization =
            hyper::header::HeaderValue::from_str(&format!("Bearer {}", grant.token.as_str()))
                .map_err(|_| Rejection::Other("invalid relay grant".into()))?;
        authorization.set_sensitive(true);
        let request = Request::post(self.endpoint.clone())
            .header(hyper::header::AUTHORIZATION, authorization)
            .header(hyper::header::CONTENT_TYPE, "application/json")
            .body(Full::new(Bytes::from(body.to_string())))
            .map_err(|_| Rejection::Other("invalid relay request".into()))?;
        let response = self
            .client
            .request(request)
            .await
            .map_err(|_| transient("relay network unavailable", 60, false))?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(hyper::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(retry_seconds);
        let mut body = response.into_body();
        let mut bytes = Vec::new();
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|_| transient("relay response interrupted", 60, false))?;
            if let Ok(data) = frame.into_data() {
                if bytes.len() + data.len() > MAX_RESPONSE {
                    return classify(status, None, retry_after, crate::now_ms() / 1000);
                }
                bytes.extend_from_slice(&data);
            }
        }
        let reply = serde_json::from_slice::<Reply>(&bytes)
            .ok()
            .filter(|v| v.v == 1);
        classify(status, reply.as_ref(), retry_after, crate::now_ms() / 1000)
    }
}

impl Sender for Relay {
    fn mode(&self) -> protocol::PushMode {
        protocol::PushMode::Relay
    }

    fn send<'a>(
        &'a self,
        device: &'a Device,
        alert: &'a Alert,
    ) -> BoxFuture<'a, Result<(), Rejection>> {
        Box::pin(async move {
            tokio::time::timeout(TIMEOUT, self.attempt(device, alert))
                .await
                .unwrap_or_else(|_| Err(transient("relay request timed out", 60, false)))
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    v: u8,
    result: String,
    retry_after: Option<u64>,
    reset_at: Option<u64>,
}

fn retry_seconds(value: &str) -> Option<u64> {
    value
        .parse::<u64>()
        .ok()
        .or_else(|| {
            httpdate::parse_http_date(value)
                .ok()?
                .duration_since(std::time::SystemTime::now())
                .ok()
                .map(|d| d.as_secs())
        })
        .map(|n| n.clamp(1, 3600))
}

fn transient(reason: &str, seconds: u64, daily_quota: bool) -> Rejection {
    Rejection::Retry {
        reason: reason.into(),
        after: Duration::from_secs(seconds),
        daily_quota,
    }
}

fn classify(
    status: u16,
    reply: Option<&Reply>,
    retry_after: Option<u64>,
    now: u64,
) -> Result<(), Rejection> {
    let result = reply.map(|r| r.result.as_str());
    match (status, result) {
        (200, Some("accepted")) => Ok(()),
        (410, Some("invalid_route")) => Err(Rejection::Unregistered),
        (429, Some("daily_quota")) => {
            let reset = reply
                .and_then(|r| r.reset_at)
                .filter(|t| *t > now && *t <= now.saturating_add(86400));
            Err(transient(
                "relay daily quota exhausted",
                reset.map_or(60, |t| t - now),
                true,
            ))
        }
        (502, Some("configuration_error")) => Err(Rejection::Other(
            "relay APNs configuration error; registration retained".into(),
        )),
        (429, _) | (500..=599, _) => {
            let after = retry_after
                .or_else(|| reply.and_then(|r| r.retry_after).map(|n| n.clamp(1, 3600)))
                .unwrap_or(60);
            Err(transient("relay temporarily unavailable", after, false))
        }
        (401, _) => Err(Rejection::Other(
            "relay grant unauthorized; open the paired app to renew".into(),
        )),
        (400, _) => Err(Rejection::Other(
            "relay rejected the encrypted envelope".into(),
        )),
        (300..=399, _) => Err(Rejection::Other("relay redirect refused".into())),
        // Cloudflare error 1027 can be an HTML response rather than relay JSON.
        _ if reply.is_none() => Err(transient(
            "relay returned an unexpected response",
            60,
            false,
        )),
        _ => Err(Rejection::Other(
            "relay request rejected; registration retained".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_quota_reset_and_opaque_cloudflare_failures() {
        let quota = Reply {
            v: 1,
            result: "daily_quota".into(),
            retry_after: None,
            reset_at: Some(86400),
        };
        assert_eq!(
            classify(429, Some(&quota), None, 86390),
            Err(transient("relay daily quota exhausted", 10, true))
        );
        assert_eq!(
            classify(429, Some(&quota), None, 86401),
            Err(transient("relay daily quota exhausted", 60, true))
        );
        for status in [429, 500, 502, 503, 520, 200] {
            assert!(matches!(
                classify(status, None, Some(300), 100),
                Err(Rejection::Retry { .. })
            ));
        }
        let config = Reply {
            v: 1,
            result: "configuration_error".into(),
            retry_after: None,
            reset_at: None,
        };
        assert!(matches!(
            classify(502, Some(&config), None, 100),
            Err(Rejection::Other(_))
        ));
        assert!(matches!(
            classify(410, None, None, 100),
            Err(Rejection::Retry { .. })
        ));
        let invalid = Reply {
            result: "invalid_route".into(),
            ..config
        };
        assert_eq!(
            classify(410, Some(&invalid), None, 100),
            Err(Rejection::Unregistered)
        );
    }
}

#[cfg(test)]
mod http_tests {
    use super::*;
    use base64::Engine;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn server(
        replies: Vec<(u16, String, Option<String>)>,
    ) -> (Relay, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1/send", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let connector = HttpsConnectorBuilder::new()
            .with_provider_and_platform_verifier(collie_tls::provider())
            .unwrap()
            .https_or_http()
            .enable_http1()
            .build();
        let relay = Relay {
            client: Client::builder(TokioExecutor::new()).build(connector),
            endpoint,
        };
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body, retry_after) in replies {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut bytes = [0; 4096];
                loop {
                    let read = stream.read(&mut bytes).await.unwrap();
                    assert!(read > 0);
                    request.extend_from_slice(&bytes[..read]);
                    assert!(request.len() <= 16384);
                    if let Some(header_end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                        let headers = std::str::from_utf8(&request[..header_end]).unwrap();
                        let length = headers
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .map(|n| n.parse::<usize>().unwrap())
                            })
                            .unwrap();
                        if request.len() >= header_end + 4 + length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(request).unwrap());
                let retry =
                    retry_after.map_or_else(String::new, |s| format!("Retry-After: {s}\r\n"));
                stream.write_all(format!("HTTP/1.1 {status} response\r\nContent-Length: {}\r\nConnection: close\r\n{retry}\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
            requests
        });
        (relay, server)
    }
    fn device() -> Device {
        let mut key = [0u8; 32];
        getrandom::fill(&mut key).unwrap();
        Device {
            stable_id: "phone".into(),
            token: protocol::PushToken::new("a".repeat(64)).unwrap(),
            environment: protocol::ApnsEnvironment::Production,
            notification_key: Some(
                protocol::NotificationKey::new(
                    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key),
                )
                .unwrap(),
            ),
            registered_at: crate::now_ms(),
            encrypted_display: true,
            mute_done: false,
            relay_grant: Some(protocol::RelayGrant {
                token: protocol::RelaySendToken::new(format!("v1.key.YQ.{}", "A".repeat(43)))
                    .unwrap(),
                route_id: protocol::RelayRouteId::new("route").unwrap(),
                generation: 1,
                expires_at: crate::now_ms() / 1000 + 86400,
            }),
        }
    }

    #[tokio::test]
    async fn forwards_every_encrypted_type_without_device_tokens_or_caller_headers() {
        let accepted = r#"{"v":1,"result":"accepted"}"#.to_string();
        let (relay, server) = server(vec![(200, accepted, None); 4]).await;
        let device = device();
        let activity = crate::push::Activity {
            stable_id: device.stable_id.clone(),
            activity_id: protocol::ActivityId::new("ACT").unwrap(),
            terminal_id: protocol::TerminalId::new("term").unwrap(),
            token: protocol::PushToken::new("b".repeat(160)).unwrap(),
            environment: device.environment,
            registered_at: crate::now_ms(),
            shows_approvals: true,
            display_binding: Some(protocol::ActivityDisplayBinding {
                node_id: protocol::Label::new("nMAC").unwrap(),
                binding_id: protocol::ActivityId::new("binding").unwrap(),
            }),
            relay_grant: device.relay_grant.clone(),
        };
        let alert =
            crate::push::session_test_alert("nMAC", &activity.terminal_id).for_device(&device);
        let clear = crate::push::approval_clear(
            "nMAC",
            &protocol::ApprovalId::new("A".repeat(22)).unwrap(),
        );
        let mut live = crate::activity::end(None, crate::now_ms());
        live.payload["aps"]["event"] = json!("update");
        live.delivery = Delivery::LiveActivity { urgent: false };
        let live = live.for_activity(&activity, Some(&device)).unwrap();
        let end = crate::activity::end(None, crate::now_ms())
            .for_activity(&activity, Some(&device))
            .unwrap();
        for alert in [alert, clear, live, end] {
            relay.send(&device, &alert).await.unwrap();
        }
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 4);
        for (request, kind) in
            requests
                .iter()
                .zip(["alert", "background", "liveactivity", "liveactivity"])
        {
            let (headers, body) = request.split_once("\r\n\r\n").unwrap();
            assert!(headers.starts_with("POST /v1/send HTTP/1.1\r\n"));
            assert!(headers.contains("authorization: Bearer v1.key."));
            assert!(!headers.contains("apns-"));
            assert!(!body.contains(device.token.as_str()));
            let body: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(body["push_type"], kind);
            assert!(body.get("topic").is_none() && body.get("destination").is_none());
        }
    }

    #[tokio::test]
    async fn html_quota_and_oversized_responses_retry_but_configuration_errors_keep_registration() {
        let (relay, server) = server(vec![
            (
                503,
                "<html>Cloudflare error 1027</html>".into(),
                Some("999999".into()),
            ),
            (200, "x".repeat(2048), None),
            (
                502,
                r#"{"v":1,"result":"configuration_error"}"#.into(),
                None,
            ),
            (302, r#"{"v":1,"result":"accepted"}"#.into(), None),
        ])
        .await;
        let device = device();
        let alert = crate::push::test_alert().for_device(&device);
        assert!(
            matches!(relay.send(&device,&alert).await,Err(Rejection::Retry{after,..}) if after==Duration::from_secs(3600))
        );
        assert!(matches!(
            relay.send(&device, &alert).await,
            Err(Rejection::Retry { .. })
        ));
        assert!(matches!(
            relay.send(&device, &alert).await,
            Err(Rejection::Other(_))
        ));
        assert!(matches!(
            relay.send(&device, &alert).await,
            Err(Rejection::Other(_))
        ));
        assert_eq!(server.await.unwrap().len(), 4);
    }

    #[test]
    fn retry_after_accepts_dates_and_clamps_delays() {
        assert_eq!(retry_seconds("999999"), Some(3600));
        assert_eq!(retry_seconds("0"), Some(1));
        let date = httpdate::fmt_http_date(std::time::SystemTime::now() + Duration::from_secs(120));
        assert!(retry_seconds(&date).is_some_and(|n| (118..=120).contains(&n)));
        assert_eq!(retry_seconds("not a date"), None);
    }
}
