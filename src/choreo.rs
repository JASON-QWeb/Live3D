use crate::{
    config::now,
    session::Run,
    state::{App, lock},
    wire::{self, Command, Packet},
};
use anyhow::{Result, ensure};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
pub fn enable(app: &App, yes: bool) -> Result<()> {
    let run = app.active()?;
    ensure!(
        !yes || !app.config().actions.is_empty(),
        "请先在设置中填写动作编排"
    );
    run.choreo.send_replace(yes);
    lock(&app.view).choreo = yes;
    Ok(())
}
pub fn hold(app: &App, run: &Run) {
    run.next_action.store(
        now() + app.config().action_gap as u64 * 1000,
        Ordering::Relaxed,
    );
}
pub async fn worker(app: App, run: Arc<Run>) {
    let mut on = run.choreo.subscribe();
    let mut next = 0;
    loop {
        tokio::select! {
            biased;
            _ = run.stop.cancelled() => break,
            change = on.changed() => {
                if change.is_err() { break; }
                next = 0;
            }
            _ = tokio::time::sleep(Duration::from_millis(300)), if *on.borrow() => {
                let config = app.config();
                if now() < run.next_action.load(Ordering::Relaxed)
                    || config.actions.is_empty() || lock(&app.view).phase != "live" {
                    continue;
                }
                let line = &config.actions[next % config.actions.len()];
                let packet = Packet {
                    epoch: *run.epoch.borrow(),
                    cmd: Command::Signal(9, wire::text_msg(line)),
                };
                if run.wire.try_send(packet).is_ok() {
                    app.event("编排", line.clone());
                    next = next.wrapping_add(1);
                    hold(&app, &run);
                }
            }
        }
    }
}
