//! Continuous causal calibration. No retrospective reconstruction of predictions.
use serde::{Deserialize, Serialize};
use super::personal::MaturedOutcome;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all="camelCase", default)]
pub struct AffineCalibration {
    version: u32,
    pub n: f64,
    x: f64,
    y: f64,
    xx: f64,
    xy: f64,
    feedbacks: u64,
    last_update_at: Option<i64>,
}
impl Default for AffineCalibration {
    fn default()->Self { Self{version:2,n:0.,x:0.,y:0.,xx:0.,xy:0.,feedbacks:0,last_update_at:None} }
}
impl AffineCalibration {
    pub fn valid(&self, last_at: Option<i64>)->bool {
        self.version==2 && [self.n,self.x,self.y,self.xx,self.xy].iter().all(|v| v.is_finite() && *v>=0.)
            && self.n<=100.+1e-9 && self.y<=100.*self.n+1e-9
            && (self.n>0. || [self.x,self.y,self.xx,self.xy].iter().all(|v|*v==0.))
            && self.last_update_at.is_none_or(|t|t>0 && last_at.is_some_and(|end|t<=end))
            && (self.feedbacks==0)==self.last_update_at.is_none()
            && self.x*self.x<=self.n*self.xx+1e-8*(self.n*self.xx).abs().max(1.)
    }
    pub fn rate(&self, base:f64)->Option<f64> {
        if !base.is_finite() || base<0. { return None; }
        if self.n<=0. { return Some(base); }
        let x=base/12.; let a=self.xx+10.;let b=self.n+10.;let det=a*b-self.x*self.x;
        if !det.is_finite() || det<=0. { return None; }
        let slope=(((self.xy+10.)*b-self.y*self.x)/det).clamp(0.5,1.5);
        let offset=((a*self.y-self.x*(self.xy+10.))/det).clamp(-1.5,1.5);
        let trust=self.n/(self.n+8.);
        let result=(x+trust*(slope*x+offset-x)).clamp(0.75*x,1.5*x)*12.;
        (result.is_finite() && result>=0.).then_some(result)
    }
    pub fn learn(&mut self, base:f64, f:&MaturedOutcome, at:i64) {
        let x=base/12.;let y=f.observed_delta*300./f.elapsed_seconds as f64;
        if !x.is_finite() || !y.is_finite() || x<0. || y<0. || !(300..=390).contains(&f.elapsed_seconds)
            || !(x*x).is_finite() || !(x*y).is_finite() { return; }
        self.n*=0.99;self.x*=0.99;self.y*=0.99;self.xx*=0.99;self.xy*=0.99;
        let raw=base*f.elapsed_seconds as f64/3600.;
        if f.confidence>0. && raw<=f.fixed_predicted_delta+1e-8 {
            self.n+=1.;self.x+=x;self.y+=y;self.xx+=x*x;self.xy+=x*y;
        }
        self.feedbacks=self.feedbacks.saturating_add(1);self.last_update_at=Some(at);
    }
}
pub fn recover<'de,D:serde::Deserializer<'de>>(d:D)->Result<AffineCalibration,D::Error>{
    let value=serde_json::Value::deserialize(d)?;Ok(serde_json::from_value(value).unwrap_or_default())
}
pub fn recover_rate<'de,D:serde::Deserializer<'de>>(d:D)->Result<Option<f64>,D::Error>{
    let v=serde_json::Value::deserialize(d)?;Ok(v.as_f64().filter(|v|v.is_finite() && *v>=0.))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn feedback()->MaturedOutcome { MaturedOutcome{elapsed_seconds:300,predicted_delta:3.,observed_delta:4.,confidence:1.,published_predicted_delta:Some(3.),fixed_predicted_delta:3.,shadow_predicted_delta:Some(3.)} }
    #[test]
    fn neutral_and_trained_rates_are_continuous_monotone_and_bounded() {
        let mut model=AffineCalibration::default();assert_eq!(model.rate(36.),Some(36.));
        for i in 1..30 { model.learn(36.,&feedback(),1000+i*300); }
        assert!(model.valid(Some(10000)));let mut previous=0.;
        for i in 0..10000 {let base=i as f64/100.;let rate=model.rate(base).unwrap();assert!(rate>=previous);assert!(rate>=base*0.75-1e-9 && rate<=base*1.5+1e-9);assert!(rate-previous<=0.016);previous=rate;}
    }
    #[test]
    fn capped_and_uninformative_feedback_only_decay_and_invalid_models_are_detected() {
        let mut model=AffineCalibration::default();model.learn(36.,&feedback(),1000);let n=model.n;
        let mut f=feedback();f.fixed_predicted_delta=2.;model.learn(36.,&f,1300);assert_eq!(model.n,n*0.99);
        f=feedback();f.confidence=0.;let n=model.n;model.learn(36.,&f,1600);assert_eq!(model.n,n*0.99);
        let mut bad=model.clone();bad.version=99;assert!(!bad.valid(Some(1600)));
        let restored:AffineCalibration=serde_json::from_value(serde_json::to_value(&model).unwrap()).unwrap();assert_eq!(restored.rate(36.),model.rate(36.));
    }
}

#[cfg(test)]
mod cross_language_tests {
    use super::*;
    #[test]
    fn every_synthetic_origin_matches_the_independent_javascript_trajectory() {
        let csv=include_str!("../../../tests/fixtures/affine-parity.csv");let mut model=AffineCalibration::default();let mut count=0;
        for line in csv.lines().skip(1) {
            let v:Vec<f64>=line.split(',').map(|v|v.parse().unwrap()).collect();
            let actual=model.rate(v[0]).unwrap();assert!((actual-v[6]).abs()<=1e-9*v[6].abs().max(1.));
            let f=MaturedOutcome{elapsed_seconds:v[1] as i64,predicted_delta:v[4],observed_delta:v[2],confidence:v[3],published_predicted_delta:None,fixed_predicted_delta:v[4],shadow_predicted_delta:None};
            model.learn(v[0],&f,v[5] as i64);assert!(model.valid(Some(v[5] as i64)));count+=1;
        }
        assert_eq!(count,96);
    }
}
