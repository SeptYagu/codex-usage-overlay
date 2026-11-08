//! Frozen sample candidates and pure display selection; never accepts observations.
use crate::{codex::CodexUsage, config::FiveHourBurnRateAlgorithm as Algorithm};
use serde::Serialize;

#[derive(Clone, Debug)]
pub struct Candidates { pub legacy:Option<f64>, pub weighted:Option<f64>, pub personalized:Option<f64>, pub learning:&'static str }
#[derive(Clone, Debug)]
pub struct Sample { pub usage:CodexUsage, pub candidates:Candidates }
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all="camelCase")]
pub struct Metadata {
    pub presentation_revision:u64,
    pub settings_revision:u64,
    pub requested_algorithm:Option<Algorithm>,
    pub effective_algorithm:Option<Algorithm>,
    pub fallback_reason:Option<&'static str>,
    pub personal_learning_status:Option<&'static str>,
}
#[derive(Default)]
pub struct Cache { pub source:Option<Sample>, pub current:Option<CodexUsage>, pub metadata:Metadata }

impl Sample {
    pub fn project(&self, requested:Algorithm)->(CodexUsage,Option<Algorithm>,Option<&'static str>) {
        let c=&self.candidates;
        let options=match requested {
            Algorithm::Legacy=>vec![(Algorithm::Legacy,c.legacy)],
            Algorithm::Weighted=>vec![(Algorithm::Weighted,c.weighted),(Algorithm::Legacy,c.legacy)],
            Algorithm::Personalized=>vec![(Algorithm::Personalized,c.personalized),(Algorithm::Weighted,c.weighted),(Algorithm::Legacy,c.legacy)],
        };
        let selected=options.into_iter().find(|(_,v)|v.is_some_and(|v|v.is_finite() && v>=0.));
        let mut usage=self.usage.clone();usage.five_hour_burn_rate_per_hour=selected.and_then(|(_,v)|v);
        let effective=selected.map(|(a,_)|a);
        let reason=if effective==Some(requested){None}else if c.learning=="identityUnavailable" {Some("identityUnavailable")} else {Some(match effective{None=>"unavailable",Some(Algorithm::Weighted)=>"personalizedUnavailable",_=>"weightedUnavailable"})};
        (usage,effective,reason)
    }
}
impl Cache {
    pub fn project(&mut self, requested:Algorithm, settings_revision:u64, slow:bool)->Option<CodexUsage> {
        let source=self.source.as_ref()?;let (usage,effective,reason)=source.project(requested);
        self.metadata=Metadata{presentation_revision:self.metadata.presentation_revision+1,settings_revision,
            requested_algorithm:Some(requested),effective_algorithm:effective,fallback_reason:reason,
            personal_learning_status:Some(if slow && matches!(source.candidates.learning,"active"|"accumulating") {"observationGap"}else{source.candidates.learning})};
        self.current=Some(usage.clone());
        Some(usage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selector_preserves_zero_and_weekly_and_reports_honest_fallback() {
        let usage:CodexUsage=serde_json::from_value(serde_json::json!({"fiveHourRemainingPercent":80,"fiveHourBurnRatePerHour":1,"weekRemainingPercent":90,"weekBurnRatePerHour":7,"creditsDisplay":"0","creditsBalance":null,"hasCredits":false,"fiveHourResetsAt":2000,"weekResetsAt":3000,"fetchedAt":1000})).unwrap();
        let mut sample=Sample{usage,candidates:Candidates{legacy:Some(1.),weighted:Some(2.),personalized:Some(0.),learning:"active"}};
        let (u,a,r)=sample.project(Algorithm::Personalized);assert_eq!(u.five_hour_burn_rate_per_hour,Some(0.));assert_eq!(a,Some(Algorithm::Personalized));assert_eq!(r,None);assert_eq!(u.week_burn_rate_per_hour,Some(7.));assert_eq!(u.fetched_at,1000);
        sample.candidates.personalized=None;assert_eq!(sample.project(Algorithm::Personalized).1,Some(Algorithm::Weighted));
        sample.candidates.weighted=Some(f64::NAN);assert_eq!(sample.project(Algorithm::Weighted).1,Some(Algorithm::Legacy));
        sample.candidates.legacy=None;assert_eq!(sample.project(Algorithm::Personalized).1,None);
    }
}

/// Caller holds this runtime's settings_patch commit gate. No reader/learner is touched.
pub async fn reproject_locked<S:crate::tray::TrayPresentationSink>(sink:&S,state:&std::sync::Arc<crate::commands::AppState>) {
    let settings=state.settings.lock().await.clone();
    let usage=state.presentation.lock().unwrap_or_else(|p|p.into_inner()).project(settings.five_hour_burn_rate_algorithm,
        state.applied_settings_revision.load(std::sync::atomic::Ordering::Acquire),state.polling.lock().unwrap_or_else(|p|p.into_inner()).healthy_interval_seconds(&settings)>90);
    let Some(usage)=usage else{return;};
    *state.last_usage.lock().await=Some(usage.clone());
    let status=*state.tray_read_status.lock().await;
    {
        let _tray=state.tray_gate.lock().unwrap_or_else(|p|p.into_inner());
        if state.is_displayed(){sink.present(crate::tray::runtime_presentation(state,&settings,Some(&usage),status));}
    }
    sink.publish_usage(crate::commands::usage_envelope(state,&settings,usage));
    if let Some(app)=sink.app_handle(){
        if let Some(owner)=state.owner.get().and_then(|w|w.upgrade()){
            if let Some(registry)=owner.monitors.get(){registry.publish(app).await;}
        }
    }
}
