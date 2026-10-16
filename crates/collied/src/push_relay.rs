use super::*;
use tokio::time::Instant;

const ROUTINE_GAP: Duration = Duration::from_secs(60);
const MAX_RETRY_AGE: Duration = Duration::from_secs(120);

struct Pending {
    device: Device,
    alert: Alert,
    activity: Option<Activity>,
    approval: Option<String>,
    fallback: Option<Box<Routed>>,
    due: Instant,
    expires: Instant,
    attempts: u8,
}

impl Pending {
    fn routine(&self) -> bool {
        self.alert.delivery == (Delivery::LiveActivity { urgent: false })
            && self.alert.payload["aps"]["event"] == "update"
    }

    fn same_activity(&self, other: &Self) -> bool {
        self.device.stable_id == other.device.stable_id
            && self
                .activity
                .as_ref()
                .zip(other.activity.as_ref())
                .is_some_and(|(a, b)| {
                    a.activity_id == b.activity_id || a.terminal_id == b.terminal_id
                })
    }

    fn ready_at(
        &self,
        cooldown: Instant,
        routine_cooldown: Instant,
        cadence: &VecDeque<(String, TerminalId, Instant)>,
    ) -> Instant {
        let due = self.due.max(cooldown);
        if !self.routine() {
            return due;
        }
        let last = self.activity.as_ref().and_then(|a| {
            cadence
                .iter()
                .find(|(id, t, _)| *id == a.stable_id && *t == a.terminal_id)
                .map(|(_, _, at)| *at)
        });
        due.max(routine_cooldown)
            .max(last.map_or(due, |at| at + ROUTINE_GAP))
    }

    fn valid(&self, push: &Push, now: Instant) -> bool {
        if now >= self.expires
            || !(push.paired)(&self.device.stable_id)
            || self
                .device
                .relay_grant
                .as_ref()
                .is_none_or(|g| !g.is_valid(crate::now_ms() / 1000))
            || self
                .alert
                .expiration
                .is_some_and(|t| t != 0 && t <= crate::now_ms() / 1000)
            || self.approval.as_ref().is_some_and(|id| {
                crate::lock(&push.approval_live)
                    .as_ref()
                    .is_some_and(|check| !check(id))
            })
        {
            return false;
        }
        let devices = push.lock();
        if let Some(activity) = &self.activity {
            if self.alert.payload["aps"]["event"] == "end" {
                return devices
                    .devices
                    .iter()
                    .any(|d| d.stable_id == self.device.stable_id)
                    && !devices.activities.iter().any(|a| {
                        a.stable_id == activity.stable_id
                            && (a.activity_id == activity.activity_id
                                || a.terminal_id == activity.terminal_id)
                            && (a.token != activity.token || a.relay_grant != activity.relay_grant)
                    });
            }
            devices.activities.iter().any(|a| {
                a.stable_id == activity.stable_id
                    && a.activity_id == activity.activity_id
                    && a.token == self.device.token
                    && a.relay_grant == self.device.relay_grant
            })
        } else {
            devices.devices.iter().any(|d| {
                d.stable_id == self.device.stable_id
                    && d.token == self.device.token
                    && d.relay_grant == self.device.relay_grant
            })
        }
    }
}

fn pending(
    device: Device,
    alert: Alert,
    activity: Option<Activity>,
    approval: Option<String>,
    fallback: Option<Box<Routed>>,
) -> Pending {
    let now = Instant::now();
    let lifetime = approval
        .as_ref()
        .and(alert.expiration)
        .filter(|t| *t > 0)
        .map(|t| {
            Duration::from_secs(t.saturating_sub(crate::now_ms() / 1000))
                .min(Duration::from_secs(3600))
        })
        .unwrap_or(MAX_RETRY_AGE);
    Pending {
        device,
        alert,
        activity,
        approval,
        fallback,
        due: now,
        expires: now + lifetime,
        attempts: 0,
    }
}

fn add(queue: &mut Vec<Pending>, next: Pending) {
    if next.activity.is_some() {
        queue.retain(|old| !old.same_activity(&next) || (next.routine() && !old.routine()));
    } else if next.alert.delivery == Delivery::Background {
        let id = &next.alert.payload["approval_id"];
        queue.retain(|old| {
            old.device.stable_id != next.device.stable_id || old.alert.payload["approval_id"] != *id
        });
    } else if let Some(id) = &next.alert.collapse_id {
        queue.retain(|old| {
            old.device.stable_id != next.device.stable_id
                || old.alert.collapse_id.as_ref() != Some(id)
        });
    }
    if queue.len() == QUEUE {
        if next.routine() && !queue.iter().any(Pending::routine) {
            return;
        }
        let victim = queue.iter().position(Pending::routine).unwrap_or(0);
        queue.remove(victim);
    }
    queue.push(next);
}

fn enqueue(
    push: &Push,
    queue: &mut Vec<Pending>,
    job: Job,
    alerted: &mut VecDeque<(String, ApprovalId)>,
) {
    match job {
        Job::Alert(devices, alert) => {
            for device in devices {
                let approval = (alert.delivery == Delivery::Alert)
                    .then(|| alert.payload["approval_id"].as_str().map(str::to_owned))
                    .flatten();
                add(
                    queue,
                    pending(
                        device.clone(),
                        alert.for_device(&device),
                        None,
                        approval,
                        None,
                    ),
                );
            }
        }
        Job::Activity(activity, alert, fallback) => {
            let device = push
                .devices()
                .into_iter()
                .find(|d| d.stable_id == activity.stable_id);
            let approval = fallback
                .as_ref()
                .map(|r| r.approval_id.as_str().to_owned())
                .or_else(|| {
                    alert.payload["aps"]["content-state"]["approvalId"]
                        .as_str()
                        .map(str::to_owned)
                });
            if let Some(alert) = alert.for_activity(&activity, device.as_ref()) {
                add(
                    queue,
                    pending(
                        activity.target(),
                        alert,
                        Some(*activity),
                        approval,
                        fallback,
                    ),
                );
            } else if let Some(fallback) = fallback {
                enqueue(push, queue, Job::Routed(fallback), alerted);
            }
        }
        Job::Routed(r) => {
            let key = (r.device.stable_id.clone(), r.approval_id.clone());
            if alerted.contains(&key) {
                return;
            }
            if alerted.len() == QUEUE {
                alerted.pop_front();
            }
            alerted.push_back(key);
            let alert = r.alert.for_device(&r.device);
            add(
                queue,
                pending(
                    r.device,
                    alert,
                    None,
                    Some(r.approval_id.as_str().to_owned()),
                    None,
                ),
            );
        }
    }
}

fn wall_time(at: Instant) -> u64 {
    crate::now_ms().saturating_add(at.saturating_duration_since(Instant::now()).as_millis() as u64)
}

pub(super) async fn deliver(
    owner: Weak<Push>,
    mut rx: mpsc::Receiver<Job>,
    sender: Arc<dyn Sender>,
) {
    let mut queue = Vec::new();
    let mut alerted = VecDeque::new();
    let mut cadence: VecDeque<(String, protocol::TerminalId, Instant)> = VecDeque::new();
    let mut cooldown = Instant::now();
    let mut routine_cooldown = cooldown;
    loop {
        let Some(push) = owner.upgrade() else {
            return;
        };
        for _ in 0..QUEUE {
            let Ok(job) = rx.try_recv() else {
                break;
            };
            enqueue(&push, &mut queue, job, &mut alerted);
        }
        let mut fallback = Vec::new();
        queue.retain_mut(|job: &mut Pending| {
            let valid = job.valid(&push, Instant::now());
            if !valid && let Some(routed) = job.fallback.take() {
                fallback.push(routed);
            }
            valid
        });
        for routed in fallback {
            enqueue(&push, &mut queue, Job::Routed(routed), &mut alerted);
        }
        cadence.retain(|(_, _, at)| Instant::now().duration_since(*at) < ROUTINE_GAP);
        let next = queue
            .iter()
            .enumerate()
            .min_by_key(|(_, p)| {
                (
                    p.ready_at(cooldown, routine_cooldown, &cadence),
                    p.routine(),
                )
            })
            .map(|(i, p)| (i, p.ready_at(cooldown, routine_cooldown, &cadence)));
        {
            let mut health = crate::lock(&push.health);
            health.pending = queue.len();
            health.retry_at_ms = next.map(|(_, at)| wall_time(at));
            if health
                .quota_reset_at_ms
                .is_some_and(|at| at <= crate::now_ms())
            {
                health.quota_reset_at_ms = None;
            }
        }
        if next.is_none_or(|(_, due)| due > Instant::now()) {
            let due =
                next.map(|(_, at)| at.min(queue.iter().map(|p| p.expires).min().unwrap_or(at)));
            drop(push);
            tokio::select! {
                job = rx.recv() => match job {
                    Some(job) => {
                        let Some(push) = owner.upgrade() else { return; };
                        enqueue(&push,&mut queue,job,&mut alerted);
                        for _ in 1..QUEUE {
                            let Ok(job) = rx.try_recv() else { break; };
                            enqueue(&push,&mut queue,job,&mut alerted);
                        }
                    }
                    None => return,
                },
                () = async { match due { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => {}
            }
            continue;
        }
        let (index, _) = next.unwrap();
        let mut job = queue.remove(index);
        if !job.valid(&push, Instant::now()) {
            continue;
        }
        if let Some(a) = &job.activity {
            cadence.retain(|(id, t, _)| *id != a.stable_id || *t != a.terminal_id);
            if cadence.len() == QUEUE {
                routine_cooldown = routine_cooldown.max(Instant::now() + ROUTINE_GAP);
                cadence.pop_front();
            }
            cadence.push_back((a.stable_id.clone(), a.terminal_id.clone(), Instant::now()));
        }
        let result = sender.send(&job.device, &job.alert).await;
        let mut health = crate::lock(&push.health);
        health.last_send_ms = Some(crate::now_ms());
        health.last_error = result.as_ref().err().map(ToString::to_string);
        drop(health);
        match result {
            Ok(()) => continue,
            Err(Rejection::Unregistered | Rejection::BadDeviceToken) => {
                push.drop_token(&job.device)
            }
            Err(Rejection::Retry {
                after, daily_quota, ..
            }) => {
                job.attempts = job.attempts.saturating_add(1);
                let backoff = Duration::from_secs(2u64.pow(u32::from(job.attempts.min(8))));
                let mut random = [0u8; 2];
                let _ = getrandom::fill(&mut random);
                let jitter = Duration::from_millis(u64::from(u16::from_le_bytes(random)) % 1000);
                let at =
                    Instant::now() + after.max(backoff).min(Duration::from_secs(86400)) + jitter;
                if daily_quota {
                    crate::lock(&push.health).quota_reset_at_ms = Some(wall_time(at));
                    if job.routine() {
                        routine_cooldown = at;
                    } else {
                        cooldown = at;
                    }
                } else {
                    cooldown = at;
                }
                job.due = at;
                if job.fallback.is_none() && job.valid(&push, Instant::now()) {
                    queue.push(job);
                    continue;
                }
            }
            Err(Rejection::Other(_)) => {}
        }
        if let Some(fallback) = job.fallback.take() {
            enqueue(&push, &mut queue, Job::Routed(fallback), &mut alerted);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Default)]
    struct Mock {
        sent: Mutex<Vec<(Device, Alert)>>,
        replies: Mutex<VecDeque<Result<(), Rejection>>>,
    }

    impl Sender for Mock {
        fn mode(&self) -> protocol::PushMode {
            protocol::PushMode::Relay
        }
        fn send<'a>(
            &'a self,
            device: &'a Device,
            alert: &'a Alert,
        ) -> BoxFuture<'a, Result<(), Rejection>> {
            Box::pin(async move {
                crate::lock(&self.sent).push((device.clone(), alert.clone()));
                crate::lock(&self.replies).pop_front().unwrap_or(Ok(()))
            })
        }
    }

    fn key() -> NotificationKey {
        NotificationKey::new(URL_SAFE_NO_PAD.encode([7; 32])).unwrap()
    }
    fn grant(generation: u32) -> protocol::RelayGrant {
        protocol::RelayGrant {
            token: protocol::RelaySendToken::new(format!("v1.key.YQ.{}", "A".repeat(43))).unwrap(),
            route_id: protocol::RelayRouteId::new("route").unwrap(),
            generation,
            expires_at: crate::now_ms() / 1000 + 86400,
        }
    }
    fn register(push: &Push, generation: u32) {
        push.register(
            "phone",
            PushToken::new("a".repeat(64)).unwrap(),
            ApnsEnvironment::Production,
            key(),
            false,
            Some(grant(generation)),
        )
        .unwrap();
    }
    fn activity(push: &Push) -> Activity {
        push.register_activity(
            "phone",
            ActivityId::new("ACT").unwrap(),
            TerminalId::new("term").unwrap(),
            PushToken::new("b".repeat(160)).unwrap(),
            true,
            Some(protocol::ActivityDisplayBinding {
                node_id: protocol::Label::new("nMAC").unwrap(),
                binding_id: ActivityId::new("binding").unwrap(),
            }),
            Some(grant(1)),
        )
        .unwrap();
        push.activities()[0].clone()
    }
    fn update(title: &str, urgent: bool) -> Alert {
        Alert {
            payload: json!({"aps":{"event":"update","timestamp":crate::now_ms()/1000,
            "content-state":{"status":"working","statusSince":0,"title":title,"approvals":0}}}),
            collapse_id: None,
            expiration: Some(0),
            context: None,
            delivery: Delivery::LiveActivity { urgent },
        }
    }
    fn retry(after: u64, daily_quota: bool) -> Rejection {
        Rejection::Retry {
            reason: "unavailable".into(),
            after: Duration::from_secs(after),
            daily_quota,
        }
    }
    async fn settle() {
        for _ in 0..30 {
            tokio::task::yield_now().await;
        }
    }
    fn rig(mock: Arc<Mock>, paired: Paired) -> (tempfile::TempDir, Arc<Push>) {
        let dir = tempfile::tempdir().unwrap();
        let push = Push::open(dir.path().join("push.json"), Some(mock), paired).unwrap();
        register(&push, 1);
        (dir, push)
    }

    #[tokio::test(start_paused = true)]
    async fn routine_updates_keep_only_newest_and_preserve_urgent_cadence() {
        let mock = Arc::new(Mock::default());
        let (_dir, push) = rig(mock.clone(), Arc::new(|_| true));
        let activity = activity(&push);
        push.notify_activity(activity.clone(), update("first", false), None);
        settle().await;
        assert_eq!(crate::lock(&mock.sent).len(), 1);
        for i in 0..500 {
            push.notify_activity(
                activity.clone(),
                update(&format!("latest-{i}"), false),
                None,
            );
            settle().await;
        }
        assert_eq!(crate::lock(&mock.sent).len(), 1);
        assert_eq!(push.status().health.pending, 1);
        tokio::time::advance(Duration::from_secs(59)).await;
        settle().await;
        assert_eq!(crate::lock(&mock.sent).len(), 1);
        tokio::time::advance(Duration::from_secs(1)).await;
        settle().await;
        let sent = crate::lock(&mock.sent).clone();
        assert_eq!(sent.len(), 2);
        let alert = &sent[1].1;
        let plain = crate::push_display::open(
            &key(),
            alert.payload["aps"]["content-state"]["enc"]
                .as_str()
                .unwrap(),
            2,
            "nMAC",
            "binding",
            alert.payload["event_id"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(plain["state"]["title"], "latest-499");
        push.notify_activity(activity.clone(), update("urgent", true), None);
        settle().await;
        assert_eq!(crate::lock(&mock.sent).len(), 3);
        push.notify_activity(activity, update("after urgent", false), None);
        settle().await;
        assert_eq!(crate::lock(&mock.sent).len(), 3);
        tokio::time::advance(ROUTINE_GAP).await;
        settle().await;
        assert_eq!(crate::lock(&mock.sent).len(), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn transient_outage_is_bounded_and_recovers_without_blocking_registration() {
        let mock = Arc::new(Mock::default());
        crate::lock(&mock.replies).push_back(Err(retry(60, false)));
        let (dir, push) = rig(mock.clone(), Arc::new(|_| true));
        push.notify(test_alert());
        settle().await;
        assert_eq!(crate::lock(&mock.sent).len(), 1);
        for i in 0..1000 {
            let mut alert = test_alert();
            alert.collapse_id = Some(format!("event{i}"));
            alert.payload["aps"]["thread-id"] = json!(format!("event{i}"));
            push.notify(alert);
            settle().await;
        }
        assert_eq!(crate::lock(&mock.sent).len(), 1);
        assert_eq!(push.status().health.pending, QUEUE);
        let stored = std::fs::read_to_string(dir.path().join("push.json")).unwrap();
        assert!(!stored.contains("payload") && !stored.contains("event999"));
        register(&push, 2);
        assert_eq!(
            push.devices()[0].relay_grant.as_ref().unwrap().generation,
            2
        );
        tokio::time::advance(Duration::from_secs(61)).await;
        settle().await;
        assert_eq!(
            crate::lock(&mock.sent).len(),
            1,
            "old generation is cancelled before retries"
        );
        push.notify(test_alert());
        settle().await;
        assert_eq!(crate::lock(&mock.sent).len(), 2);
        assert!(push.status().health.last_error.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn retries_stop_when_pairing_approval_or_grant_is_no_longer_valid() {
        for invalidation in 0..3 {
            let mock = Arc::new(Mock::default());
            crate::lock(&mock.replies).push_back(Err(retry(10, false)));
            let authorized = Arc::new(AtomicBool::new(true));
            let paired = authorized.clone();
            let (_dir, push) = rig(
                mock.clone(),
                Arc::new(move |_| paired.load(Ordering::SeqCst)),
            );
            let approval = Arc::new(AtomicBool::new(true));
            let live = approval.clone();
            push.set_approval_check(Arc::new(move |_| live.load(Ordering::SeqCst)));
            let mut alert = test_alert();
            alert.payload["approval_id"] = json!("approval");
            alert.expiration = Some(crate::now_ms() / 1000 + 300);
            push.notify(alert);
            settle().await;
            assert_eq!(crate::lock(&mock.sent).len(), 1);
            match invalidation {
                0 => authorized.store(false, Ordering::SeqCst),
                1 => approval.store(false, Ordering::SeqCst),
                _ => push
                    .update(|devices| {
                        devices.devices[0].relay_grant.as_mut().unwrap().expires_at = 0
                    })
                    .unwrap(),
            }
            tokio::time::advance(Duration::from_secs(12)).await;
            settle().await;
            assert_eq!(crate::lock(&mock.sent).len(), 1);
            assert_eq!(push.status().health.pending, 0);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn newest_routine_recovers_after_backoff_and_daily_quota_preserves_urgent_reserve() {
        let mock = Arc::new(Mock::default());
        crate::lock(&mock.replies).push_back(Err(retry(10, true)));
        let (_dir, push) = rig(mock.clone(), Arc::new(|_| true));
        let activity = activity(&push);
        push.notify_activity(activity.clone(), update("before", false), None);
        settle().await;
        assert_eq!(crate::lock(&mock.sent).len(), 1);
        assert!(push.status().health.quota_reset_at_ms.is_some());
        push.notify_activity(activity.clone(), update("latest", false), None);
        push.notify(test_alert());
        settle().await;
        assert_eq!(
            crate::lock(&mock.sent).len(),
            2,
            "urgent app alerts can use reserved total slots"
        );
        tokio::time::advance(Duration::from_secs(61)).await;
        settle().await;
        assert_eq!(crate::lock(&mock.sent).len(), 3);
        let alert = &crate::lock(&mock.sent)[2].1;
        let plain = crate::push_display::open(
            &key(),
            alert.payload["aps"]["content-state"]["enc"]
                .as_str()
                .unwrap(),
            2,
            "nMAC",
            "binding",
            alert.payload["event_id"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(plain["state"]["title"], "latest");
    }

    #[tokio::test(start_paused = true)]
    async fn activity_failure_falls_back_and_late_invalidation_never_erases_new_generation() {
        let mock = Arc::new(Mock::default());
        crate::lock(&mock.replies)
            .extend([Err(Rejection::Other("configuration error".into())), Ok(())]);
        let (_dir, push) = rig(mock.clone(), Arc::new(|_| true));
        let activity = activity(&push);
        let routed = Routed {
            device: push.devices()[0].clone(),
            terminal_id: activity.terminal_id.clone(),
            approval_id: ApprovalId::new("A".repeat(22)).unwrap(),
            alert: test_alert(),
            due: true,
        };
        push.notify_activity(activity.clone(), update("urgent", true), Some(routed));
        settle().await;
        {
            let calls = crate::lock(&mock.sent);
            assert_eq!(calls.len(), 2);
            assert_eq!(calls[0].1.delivery, Delivery::LiveActivity { urgent: true });
            assert_eq!(calls[1].1.delivery, Delivery::Alert);
        }
        assert_eq!(push.activities().len(), 1);
        let successful = Routed {
            device: push.devices()[0].clone(),
            terminal_id: activity.terminal_id.clone(),
            approval_id: ApprovalId::new("B".repeat(22)).unwrap(),
            alert: test_alert(),
            due: true,
        };
        push.notify_activity(activity, update("successful", true), Some(successful));
        settle().await;
        assert_eq!(
            crate::lock(&mock.sent).len(),
            3,
            "successful activity delivery does not send its fallback"
        );
        let old = push.devices()[0].clone();
        register(&push, 2);
        push.drop_token(&old);
        assert_eq!(push.devices().len(), 1);
        assert_eq!(
            push.devices()[0].relay_grant.as_ref().unwrap().generation,
            2
        );
    }
}
