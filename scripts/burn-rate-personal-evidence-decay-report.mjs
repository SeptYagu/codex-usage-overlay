import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';

const input = resolve(process.argv[2] ?? 'output/burn-rate-replay/real-forecasts.json');
const output = resolve(process.argv[3] ?? 'output/burn-rate-replay/personal-evidence-decay');
const data = JSON.parse(readFileSync(input, 'utf8'));
const variants = data.variants.map(v => v.name);
const fixedIndex = variants.indexOf('fixed');
if (fixedIndex < 0) throw new Error('fixed variant not found');

const learner = { lambda: 0.99, prior: 10, lo: 0.75, hi: 1.5, K: 8 };
const policies = [
  { key: 'cumulative', type: 'cumulative' },
  { key: 'rho_0.95', type: 'exp', rho: 0.95 },
  { key: 'rho_0.98', type: 'exp', rho: 0.98 },
  { key: 'rho_0.99', type: 'exp', rho: 0.99 },
  { key: 'rho_0.995', type: 'exp', rho: 0.995 },
  { key: 'hard_100', type: 'hard', window: 100 },
];

const clamp = (x, lo, hi) => Math.max(lo, Math.min(hi, x));
const mean = xs => xs.length ? xs.reduce((a,b) => a + b, 0) / xs.length : null;
const quantile = (xs, p) => xs.length
  ? [...xs].sort((a,b) => a - b)[Math.ceil(xs.length * p) - 1] : null;
function feedbackConfidence(predicted, observed) {
  return observed === 0 && predicted < 0.75 ? 0 : 1;
}
function makeEvidence(policy) {
  return { policy, value: 0, queue: [] };
}
function updateEvidence(state, q) {
  const p = state.policy;
  if (p.type === 'cumulative') {
    state.value += q;
  } else if (p.type === 'exp') {
    state.value = p.rho * state.value + q;
  } else {
    state.queue.push(q);
    if (state.queue.length > p.window) state.queue.shift();
    state.value = state.queue.reduce((a,b) => a + b, 0);
  }
}
function trustFromEvidence(e) {
  return e <= 0 ? 0 : e / (e + learner.K);
}
function effectiveFactor(rawFactor, evidence) {
  const g = trustFromEvidence(evidence);
  return { trust: g, factor: 1 + g * (rawFactor - 1) };
}

const grouped = new Map();
for (const row of data.forecasts) {
  if (row.quota !== '5H') continue;
  const key = String(row.session);
  if (!grouped.has(key)) grouped.set(key, []);
  grouped.get(key).push(row);
}
for (const rows of grouped.values())
  rows.sort((a,b) => a.at - b.at || a.horizon - b.horizon || a.target_at - b.target_at);
function realReplay(policy) {
  const out = [];
  for (const rows of grouped.values()) {
    const learningRows = rows
      .filter(r => r.horizon === 300 && r.predictions[fixedIndex] !== null)
      .sort((a,b) => a.target_at - b.target_at || a.at - b.at);
    let next = 0, A = learner.prior, B = learner.prior;
    const evidence = makeEvidence(policy);
    for (const row of rows) {
      while (next < learningRows.length && learningRows[next].target_at <= row.at) {
        const learned = learningRows[next++];
        const p = learned.predictions[fixedIndex];
        const y = learned.observed_delta;
        const q = feedbackConfidence(p, y);
        A = learner.lambda * A + q * p * y;
        B = learner.lambda * B + q * p * p;
        updateEvidence(evidence, q);
      }
      const raw = clamp(B > 0 ? A / B : 1, learner.lo, learner.hi);
      const ef = effectiveFactor(raw, evidence.value);
      const fixed = row.predictions[fixedIndex];
      out.push({
        session: row.session, horizon: row.horizon, split: row.split,
        observed: row.observed_delta, fixed,
        prediction: fixed === null ? null : fixed * ef.factor,
        rawFactor: raw, evidence: evidence.value,
        trust: ef.trust, effectiveFactor: ef.factor,
      });
    }
  }
  return out;
}
function metric(rows, field) {
  const usable = rows.filter(r => r[field] !== null);
  const errors = usable.map(r => Math.abs(r[field] - r.observed));
  const bias = usable.map(r => r[field] - r.observed);
  const bySession = new Map();
  for (const r of usable) {
    if (!bySession.has(r.session)) bySession.set(r.session, []);
    bySession.get(r.session).push(Math.abs(r[field] - r.observed));
  }
  return {
    count: usable.length,
    mae: mean(errors),
    median: quantile(errors, 0.5),
    p90: quantile(errors, 0.9),
    bias: mean(bias),
    equalSessionMae: mean([...bySession.values()].map(mean)),
  };
}
function compare(rows) {
  const usable = rows.filter(r => r.fixed !== null && r.prediction !== null);
  let wins=0, losses=0;
  for (const r of usable) {
    const a=Math.abs(r.prediction-r.observed), b=Math.abs(r.fixed-r.observed);
    if (a < b-1e-9) wins++; else if (a > b+1e-9) losses++;
  }
  return { wins, losses, ties: usable.length-wins-losses };
}
const realSummary = [];
for (const policy of policies) {
  const rows = realReplay(policy);
  for (const split of ['earlier','holdout','all']) {
    const s = rows.filter(r => r.horizon === 300 && (split === 'all' || r.split === split));
    const m = metric(s, 'prediction');
    const fixed = metric(s, 'fixed');
    const cmp = compare(s);
    realSummary.push({
      policy: policy.key, split,
      ...m,
      fixedMae: fixed.mae, fixedP90: fixed.p90,
      ...cmp,
      evidenceMean: mean(s.map(r => r.evidence)),
      evidenceMax: s.length ? Math.max(...s.map(r => r.evidence)) : null,
      trustMean: mean(s.map(r => r.trust)),
      trustMax: s.length ? Math.max(...s.map(r => r.trust)) : null,
      effectiveFactorMin: s.length ? Math.min(...s.map(r => r.effectiveFactor)) : null,
      effectiveFactorMax: s.length ? Math.max(...s.map(r => r.effectiveFactor)) : null,
    });
  }
}
function scenarioValue(name, step) {
  if (name === 'stable_bias') return { p: 2, y: 2.4, target: 1.2 };
  if (name === 'bias_reversal') {
    const target = step <= 150 ? 1.2 : 0.8;
    return { p: 2, y: 2*target, target };
  }
  if (name === 'bias_removal') {
    const target = step <= 150 ? 1.2 : 1.0;
    return { p: 2, y: 2*target, target };
  }
  if (name === 'noisy_stable_bias') {
    const y = step % 2 ? 2 : 3;
    return { p: 2, y, target: 1.25 };
  }
  if (name === 'sparse_informative') {
    if (step % 5 === 0) return { p: 4, y: 4.8, target: 1.2 };
    return { p: 0.4, y: 0, target: 1.2 };
  }
  throw new Error(name);
}

function runStress(policy, scenario) {
  let A=learner.prior, B=learner.prior;
  const evidence=makeEvidence(policy);
  const rows=[];
  for (let step=1; step<=300; step++) {
    const v=scenarioValue(scenario, step);
    const raw=clamp(B>0 ? A/B : 1, learner.lo, learner.hi);
    const ef=effectiveFactor(raw, evidence.value);
    rows.push({
      step, p:v.p, y:v.y, target:v.target,
      rawFactor:raw, evidence:evidence.value,
      trust:ef.trust, effectiveFactor:ef.factor,
      prediction:v.p*ef.factor,
      absError:Math.abs(v.p*ef.factor-v.y),
    });
    const q=feedbackConfidence(v.p,v.y);
    A=learner.lambda*A + q*v.p*v.y;
    B=learner.lambda*B + q*v.p*v.p;
    updateEvidence(evidence,q);
  }
  return rows;
}
function firstSustainedWithin(rows, start, pct) {
  const candidates=rows.filter(r=>r.step>=start);
  for (let i=0;i<candidates.length;i++) {
    const tail=candidates.slice(i, Math.min(i+5,candidates.length));
    if (tail.length===5 && tail.every(r => Math.abs(r.effectiveFactor/r.target-1) <= pct))
      return candidates[i].step-start;
  }
  return null;
}
const checkpoints=[10,20,50,100,150,200,300];
const stressSummary=[];
const stressCheckpoints=[];
for (const policy of policies) {
  for (const scenario of ['stable_bias','bias_reversal','bias_removal','noisy_stable_bias','sparse_informative']) {
    const rows=runStress(policy,scenario);
    for (const step of checkpoints) {
      const r=rows[step-1];
      stressCheckpoints.push({
        policy:policy.key,scenario,step,
        evidence:r.evidence,trust:r.trust,
        rawFactor:r.rawFactor,effectiveFactor:r.effectiveFactor,
        target:r.target,absError:r.absError,
      });
    }
    const changeStart=(scenario==='bias_reversal'||scenario==='bias_removal') ? 151 : 1;
    const post=rows.filter(r=>r.step>=changeStart);
    stressSummary.push({
      policy:policy.key,scenario,
      integratedAbsError:post.reduce((a,r)=>a+r.absError,0),
      meanAbsError:mean(post.map(r=>r.absError)),
      maxFactorError:Math.max(...post.map(r=>Math.abs(r.effectiveFactor-r.target))),
      within10: firstSustainedWithin(rows,changeStart,0.10),
      within5: firstSustainedWithin(rows,changeStart,0.05),
      finalEvidence:rows.at(-1).evidence,
      finalTrust:rows.at(-1).trust,
      finalRawFactor:rows.at(-1).rawFactor,
      finalEffectiveFactor:rows.at(-1).effectiveFactor,
    });
  }
}
// Mathematical invariants and hard-window sanity.
for (const policy of policies) {
  const rows=runStress(policy,'stable_bias');
  for (const r of rows) {
    if (!(r.trust>=0 && r.trust<=1)) throw new Error(`invalid trust ${policy.key}`);
    const lo=Math.min(1,r.rawFactor)-1e-12, hi=Math.max(1,r.rawFactor)+1e-12;
    if (r.effectiveFactor<lo || r.effectiveFactor>hi)
      throw new Error(`factor amplification ${policy.key}`);
  }
}
const hard=runStress(policies.find(p=>p.key==='hard_100'),'stable_bias');
if (Math.abs(hard[100].evidence-100)>1e-12 || Math.abs(hard.at(-1).evidence-100)>1e-12)
  throw new Error('hard-100 window failed to cap at 100');

mkdirSync(output,{recursive:true});
const result={metadata:data.metadata,learner,policies,realSummary,stressSummary,stressCheckpoints};
writeFileSync(resolve(output,'result.json'),JSON.stringify(result,null,2)+'\n');
const quote=v=>`"${String(v??'').replaceAll('"','""')}"`;
function csv(name,rows){
  const cols=Object.keys(rows[0]);
  writeFileSync(resolve(output,name),[
    cols.map(quote).join(','),
    ...rows.map(r=>cols.map(c=>quote(r[c])).join(','))
  ].join('\n')+'\n');
}
csv('real-summary.csv',realSummary);
csv('stress-summary.csv',stressSummary);
csv('stress-checkpoints.csv',stressCheckpoints);
console.log(JSON.stringify({realSummary,stressSummary},null,2));
