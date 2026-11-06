//! One app-owned display coordinator. Rotation consumes caches, never readers.
use crate::{commands::AppState, config::{OverlaySettings, Provider}, tray::TrayPresentationSink};
use serde::Serialize;
use std::{collections::BTreeSet, sync::{Arc, atomic::Ordering}, time::{Duration, Instant}};
use tauri::Manager;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all="camelCase")]
pub struct Candidate { pub provider:Provider, pub source_key:String, pub source_generation:u64 }
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all="camelCase")]
pub struct Snapshot {
    pub revision:u64, pub settings_revision:u64,
    pub effective_provider:Provider, pub manual_provider:Provider,
    pub source_key:String, pub source_generation:u64,
    pub carousel_enabled:bool, pub interval_seconds:u32,
    pub candidates:Vec<Candidate>, pub pause_reasons:BTreeSet<String>, pub status:String,
}
pub struct Controller {
    snapshot:Snapshot, interactions:BTreeSet<String>,
    deadline:Option<Instant>, manual_until:Option<Instant>, resume_until:Option<Instant>,
}
impl Controller {
    pub fn new(s:&OverlaySettings)->Self {
        Self{snapshot:Snapshot{revision:0,settings_revision:0,effective_provider:s.provider,manual_provider:s.provider,
            source_key:crate::config::source_key(s),source_generation:0,carousel_enabled:s.quota_carousel_enabled,
            interval_seconds:s.quota_carousel_interval_seconds,candidates:vec![],pause_reasons:BTreeSet::new(),status:"off".into()},
            interactions:BTreeSet::new(),deadline:None,manual_until:None,resume_until:None}
    }
    fn restart(&mut self,now:Instant) {self.deadline=Some((now+Duration::from_secs(self.snapshot.interval_seconds as u64)).max(self.manual_until.unwrap_or(now)));}
    fn configure(&mut self,s:&OverlaySettings,manual:bool,now:Instant) {
        let changed=self.snapshot.carousel_enabled!=s.quota_carousel_enabled || self.snapshot.interval_seconds!=s.quota_carousel_interval_seconds;
        self.snapshot.manual_provider=s.provider;
        self.snapshot.carousel_enabled=s.quota_carousel_enabled;
        self.snapshot.interval_seconds=s.quota_carousel_interval_seconds;
        if manual || !s.quota_carousel_enabled {self.snapshot.effective_provider=s.provider;}
        if manual {self.manual_until=Some(now+Duration::from_secs(30));self.deadline=self.manual_until;}
        else if changed {self.restart(now);}
        if !s.quota_carousel_enabled {self.deadline=None;self.manual_until=None;}
    }
    fn advance(&mut self,candidates:Vec<Candidate>,mut pauses:BTreeSet<String>,now:Instant,tick:bool,resumed:bool) {
        pauses.extend(self.interactions.iter().cloned());
        let was_paused=!self.snapshot.pause_reasons.is_empty();
        let was_waiting=self.snapshot.candidates.len()<2;
        let pause_changed=self.snapshot.pause_reasons!=pauses;
        self.snapshot.candidates=candidates;
        self.snapshot.pause_reasons=pauses;
        let held=self.manual_until.is_some_and(|at|now<at);
        let paused=!self.snapshot.pause_reasons.is_empty();
        let restarted=(was_paused&&!paused)||resumed || (was_waiting&&self.snapshot.candidates.len()>=2);
        if restarted {self.restart(now);}
        if (was_paused&&!paused)||resumed {self.resume_until=self.deadline;}
        if self.snapshot.carousel_enabled && tick && !paused && !held && !restarted && !self.resume_until.is_some_and(|until|now<until) {
            let current=self.snapshot.effective_provider;
            if !self.snapshot.candidates.is_empty() && !self.snapshot.candidates.iter().any(|c|c.provider==current) {
                self.snapshot.effective_provider=self.snapshot.candidates[0].provider; self.restart(now);
            } else if self.snapshot.candidates.len()>=2 {
                if self.deadline.is_some_and(|at|now>=at) {
                    let index=self.snapshot.candidates.iter().position(|c|c.provider==current).unwrap_or(0);
                    self.snapshot.effective_provider=self.snapshot.candidates[(index+1)%self.snapshot.candidates.len()].provider;
                    self.restart(now);
                } else if self.deadline.is_none() {self.restart(now);}
            }
        }
        if paused && pause_changed {self.deadline=None;}
        self.snapshot.status=if !self.snapshot.carousel_enabled {"off"}else if paused {"paused"}else if held {"manualHold"}else if self.snapshot.candidates.len()<2 {"waiting"}else{"rotating"}.into();
    }
}
pub fn effective(state:&AppState)->Provider {Provider::ALL[state.displayed_provider.load(Ordering::Acquire) as usize]}

async fn inputs(owner:&Arc<AppState>,settings:&OverlaySettings,now:i64)->Vec<Candidate> {
    let mut result=vec![];
    let Some(registry)=owner.monitors.get() else{return result;};
    for state in registry.all() {
        let provider=state.runtime_provider.unwrap();
        if !settings.preference(provider).carousel_included || !state.monitor_enabled.load(Ordering::Acquire) || !state.is_current(){continue;}
        let scoped=state.settings.lock().await.clone();
        let blocked=state.reader.lock().unwrap_or_else(|p|p.into_inner()).failure.as_ref().is_some_and(|f|matches!(f.kind,
            crate::codex::reader::FailureKind::Authentication|crate::codex::reader::FailureKind::Permission|crate::codex::reader::FailureKind::NotConnected|crate::codex::reader::FailureKind::MissingExecutable));
        if blocked {continue;}
        let usage=state.last_usage.lock().await;
        let valid=usage.as_ref().is_some_and(|u| u.fetched_at<=now+5 && now.saturating_sub(u.fetched_at)<=(scoped.refresh_interval_seconds as i64*2).max(120)
            && (u.five_hour_remaining_percent.is_some()||u.week_remaining_percent.is_some()||(u.has_credits&&u.credits_balance.is_some())));
        if valid && state.has_reading.load(Ordering::Acquire) && state.is_current() {
            result.push(Candidate{provider,source_key:crate::config::source_key(&scoped),source_generation:state.source_generation.load(Ordering::Acquire)});
        }
    }
    result
}
fn pauses(owner:&AppState,app:Option<&tauri::AppHandle>)->BTreeSet<String> {
    let mut reasons=BTreeSet::new();let dock=owner.dock.info();
    if dock.hidden {reasons.insert("hidden".into());}
    if dock.docked&&!dock.expanded {reasons.insert("collapsed".into());}
    if owner.dock.is_interacting(){reasons.insert("interaction".into());}
    if let Some(window)=app.and_then(|a|a.get_webview_window("main")) {
        if window.is_minimized().unwrap_or(false) || !window.is_visible().unwrap_or(true) {reasons.insert("hidden".into());}
    }
    reasons
}
/// Caller owns the app settings operation gate. The display mutex itself is
/// never held over I/O; its versioned result is published after committing.
async fn update_locked<S:TrayPresentationSink>(sink:&S,owner:&Arc<AppState>,settings:&OverlaySettings,manual:Option<bool>,tick:bool,resumed:bool,now:Instant)->Snapshot {
    let candidates=inputs(owner,settings,crate::codex::reader::unix_now()).await;
    let reasons=pauses(owner,sink.app_handle());
    let identities=owner.monitors.get().map(|r|r.all()).unwrap_or_default();
    let native=sink.app_handle().and_then(|app|app.get_webview_window("main")).map(|window|{
        let visible=window.is_visible().unwrap_or(true)&&!window.is_minimized().unwrap_or(false);
        let hovered=visible&&!settings.mouse_passthrough&&window.cursor_position().ok().zip(window.outer_position().ok()).zip(window.outer_size().ok()).is_some_and(|((cursor,pos),size)|cursor.x>=pos.x as f64&&cursor.y>=pos.y as f64&&cursor.x<(pos.x as f64+size.width as f64)&&cursor.y<(pos.y as f64+size.height as f64));
        (hovered,visible&&window.is_focused().unwrap_or(false))
    });
    let (snapshot,changed,selection_changed)={
        let mut display=owner.display.lock().unwrap_or_else(|p|p.into_inner());
        let before=display.snapshot.clone();
        // Reconcile against native geometry so lost DOM leave/capture events
        // cannot freeze rotation after a drag or a hidden WebView.
        if let Some((hovered,focused))=native {
            if hovered {display.interactions.insert("hover".into());}else{display.interactions.remove("hover");}
            if !focused {display.interactions.remove("focus");}
        }
        if let Some(manual)=manual {display.configure(settings,manual,now);}
        display.advance(candidates,reasons,now,tick,resumed);
        let provider=display.snapshot.effective_provider;
        let selected=identities.iter().find(|s|s.runtime_provider==Some(provider));
        display.snapshot.source_key=crate::config::source_key(&settings.for_provider(provider));
        display.snapshot.source_generation=selected.map(|s|s.source_generation.load(Ordering::Acquire)).unwrap_or(0);
        display.snapshot.settings_revision=owner.settings_revision.load(Ordering::Acquire);
        let changed=display.snapshot!=before;
        if changed {display.snapshot.revision+=1;}
        let selection_changed=before.effective_provider!=provider || before.source_key!=display.snapshot.source_key||before.source_generation!=display.snapshot.source_generation;
        let _tray=owner.tray_gate.lock().unwrap_or_else(|p|p.into_inner());
        owner.displayed_provider.store(provider.index(),Ordering::Release);
        owner.carousel_enabled.store(display.snapshot.carousel_enabled,Ordering::Release);
        (display.snapshot.clone(),changed,selection_changed)
    };
    if selection_changed {crate::tray::repaint_tray(sink,owner).await;}
    if changed {sink.publish_display(&snapshot);}
    snapshot
}
pub async fn settings_committed<S:TrayPresentationSink>(sink:&S,owner:&Arc<AppState>,settings:&OverlaySettings,manual:bool) {
    update_locked(sink,owner,settings,Some(manual),false,false,Instant::now()).await;
    owner.display_wake.notify_one();
}
pub async fn tick<S:TrayPresentationSink>(sink:&S,owner:&Arc<AppState>,now:Instant,resumed:bool)->Snapshot {
    let _gate=owner.settings_patch.lock().await;
    let settings=owner.settings.lock().await.clone();
    update_locked(sink,owner,&settings,None,true,resumed,now).await
}
pub fn start(app:&tauri::AppHandle,owner:&Arc<AppState>) {
    let weak=Arc::downgrade(owner);let app=app.clone();
    let task=tauri::async_runtime::spawn(async move {
        let mut previous=Instant::now();
        loop {
            let Some(owner)=weak.upgrade()else{return;};
            tokio::select!{_ = tokio::time::sleep(Duration::from_millis(250))=>{},_ = owner.display_wake.notified()=>{}}
            let now=Instant::now();let resumed=now.saturating_duration_since(previous)>Duration::from_secs(2);previous=now;
            tick(&app,&owner,now,resumed).await;
        }
    });
    if let Some(registry)=owner.monitors.get(){registry.track_task(task);}
}
#[tauri::command]
pub async fn get_display_state(app:tauri::AppHandle,state:tauri::State<'_,Arc<AppState>>)->Result<Snapshot,String> {
    let _gate=state.settings_patch.lock().await;let settings=state.settings.lock().await.clone();
    Ok(update_locked(&app,state.inner(),&settings,None,false,false,Instant::now()).await)
}
#[tauri::command]
pub async fn set_display_interaction(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<AppState>>,reason:String,active:bool)->Result<(),String> {
    if window.label()!="main" || !matches!(reason.as_str(),"hover"|"focus") {return Err("Only the overlay can pause its own interactions".into());}
    let _gate=state.settings_patch.lock().await;
    {let mut display=state.display.lock().unwrap_or_else(|p|p.into_inner());if active {display.interactions.insert(reason);}else{display.interactions.remove(&reason);}}
    let settings=state.settings.lock().await.clone();
    update_locked(window.app_handle(),state.inner(),&settings,None,false,false,Instant::now()).await;
    state.display_wake.notify_one();Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn candidates()->Vec<Candidate>{Provider::ALL.into_iter().map(|provider|Candidate{provider,source_key:provider.key().into(),source_generation:0}).collect()}
    fn controller(interval:u32)->(Controller,Instant){
        let now=Instant::now();let s=OverlaySettings{quota_carousel_enabled:true,quota_carousel_interval_seconds:interval,..Default::default()};
        let mut c=Controller::new(&s);c.advance(candidates(),BTreeSet::new(),now,true,false);(c,now)
    }
    fn advance(c:&mut Controller,now:Instant){c.advance(candidates(),BTreeSet::new(),now,true,false);}
    #[test]
    fn each_interval_rotates_in_order_only_after_a_complete_dwell() {
        for interval in [3,8,30] {
            let (mut c,now)=controller(interval);let seconds=Duration::from_secs(interval as u64);
            advance(&mut c,now+seconds-Duration::from_millis(1));assert_eq!(c.snapshot.effective_provider,Provider::Codex);
            for (i,p) in [Provider::Claude,Provider::Dsh,Provider::Codex].into_iter().enumerate(){advance(&mut c,now+seconds*(i as u32+1));assert_eq!(c.snapshot.effective_provider,p);}
        }
    }
    #[test]
    fn manual_selection_holds_exactly_thirty_seconds_without_an_extra_interval() {
        let (mut c,now)=controller(8);let s=OverlaySettings{provider:Provider::Dsh,quota_carousel_enabled:true,..Default::default()};
        c.configure(&s,true,now);advance(&mut c,now+Duration::from_secs(29));assert_eq!(c.snapshot.effective_provider,Provider::Dsh);
        advance(&mut c,now+Duration::from_secs(30));assert_eq!(c.snapshot.effective_provider,Provider::Codex);
    }
    #[test]
    fn overlapping_pause_reasons_and_resume_restart_the_full_interval() {
        let (mut c,now)=controller(8);c.interactions.insert("hover".into());c.interactions.insert("focus".into());
        advance(&mut c,now+Duration::from_secs(10));c.interactions.remove("hover");advance(&mut c,now+Duration::from_secs(20));
        assert_eq!(c.snapshot.effective_provider,Provider::Codex);assert_eq!(c.snapshot.pause_reasons.len(),1);
        c.interactions.clear();advance(&mut c,now+Duration::from_secs(30));advance(&mut c,now+Duration::from_secs(37));assert_eq!(c.snapshot.effective_provider,Provider::Codex);
        advance(&mut c,now+Duration::from_secs(38));assert_eq!(c.snapshot.effective_provider,Provider::Claude);
        c.advance(candidates(),BTreeSet::new(),now+Duration::from_secs(900),true,true);assert_eq!(c.snapshot.effective_provider,Provider::Claude);
        advance(&mut c,now+Duration::from_secs(908));assert_eq!(c.snapshot.effective_provider,Provider::Dsh);
    }
    #[test]
    fn candidate_changes_preserve_current_dwell_and_zero_one_candidates_do_not_loop() {
        let (mut c,now)=controller(8);let only=vec![candidates()[1].clone()];
        c.interactions.insert("hover".into());c.advance(only.clone(),BTreeSet::new(),now,true,false);assert_eq!(c.snapshot.effective_provider,Provider::Codex);
        c.interactions.clear();c.advance(only.clone(),BTreeSet::new(),now,true,false);assert_eq!(c.snapshot.effective_provider,Provider::Codex);
        c.advance(only.clone(),BTreeSet::new(),now+Duration::from_secs(8),true,false);assert_eq!(c.snapshot.effective_provider,Provider::Claude);
        c.advance(vec![],BTreeSet::new(),now+Duration::from_secs(40),true,false);assert_eq!(c.snapshot.effective_provider,Provider::Claude);
        c.advance(only,BTreeSet::new(),now+Duration::from_secs(50),true,false);assert_eq!(c.snapshot.status,"waiting");
    }
    #[test]
    fn committed_interval_resets_deadline_without_changing_page() {
        let (mut c,now)=controller(8);let s=OverlaySettings{quota_carousel_enabled:true,quota_carousel_interval_seconds:3,..Default::default()};
        c.configure(&s,false,now+Duration::from_secs(7));advance(&mut c,now+Duration::from_secs(9));assert_eq!(c.snapshot.effective_provider,Provider::Codex);
        advance(&mut c,now+Duration::from_secs(10));assert_eq!(c.snapshot.effective_provider,Provider::Claude);
    }
}
