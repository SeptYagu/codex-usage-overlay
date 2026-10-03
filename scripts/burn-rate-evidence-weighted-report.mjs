import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';

const input = resolve(process.argv[2] ?? 'output/burn-rate-replay/real-forecasts.json');
const output = resolve(process.argv[3] ?? 'output/burn-rate-replay/evidence-weighted');
const data = JSON.parse(readFileSync(input, 'utf8'));
const variants = data.variants.map(v => v.name);
const fixedIndex = variants.indexOf('fixed');
if (fixedIndex < 0) throw new Error('fixed variant not found');

const learners = {
  '5H': { lambda: 0.99, prior: 10, lo: 0.75, hi: 1.5 },
  WK: { lambda: 0.80, prior: 0.5, lo: 0.5, hi: 2.0 },
};
const trustGrid = [
  ...[1,2,3,5,8,10,15,20].map(K => ({ type: 'update_count', K })),
  ...[1,2,3,5,8,10,15,20].map(K => ({ type: 'confidence_count', K })),
  ...[1,2,5,10,20,50,100].map(K => ({ type: 'information', K })),
];
for (const t of trustGrid) t.key = `${t.type};K=${t.K}`;

const clamp = (x, lo, hi) => Math.max(lo, Math.min(hi, x));
const mean = xs => xs.length ? xs.reduce((a,b) => a + b, 0) / xs.length : null;
const quantile = (xs, p) => xs.length
  ? [...xs].sort((a,b) => a - b)[Math.ceil(xs.length * p) - 1] : null;
function confidence(predicted, observed) {
  return observed === 0 && predicted < 0.75 ? 0 : 1;
}
function evidenceOf(row, type) {
  if (type === 'update_count') return row.evidenceUpdates;
  if (type === 'confidence_count') return row.evidenceConfidence;
  return row.evidenceInformation;
}
function trustFor(row, spec) {
  const e = evidenceOf(row, spec.type);
  return e <= 0 ? 0 : e / (e + spec.K);
}
function predictionFor(row, spec) {
  if (row.fixed === null) return null;
  const g = trustFor(row, spec);
  const effectiveFactor = 1 + g * (row.rawFactor - 1);
  return row.fixed * effectiveFactor;
}

const grouped = new Map();
for (const row of data.forecasts) {
  const key = `${row.session}|${row.quota}`;
  if (!grouped.has(key)) grouped.set(key, []);
  grouped.get(key).push(row);
}
for (const rows of grouped.values())
  rows.sort((a,b) => a.at - b.at || a.horizon - b.horizon || a.target_at - b.target_at);
function replayGroup(rows, params) {
  const learning = rows
    .filter(r => r.horizon === 300 && r.predictions[fixedIndex] !== null)
    .sort((a,b) => a.target_at - b.target_at || a.at - b.at);
  let nextLearning = 0;
  let A = params.prior, B = params.prior;
  let evidenceUpdates = 0, evidenceConfidence = 0, evidenceInformation = 0;
  const out = [];
  for (const row of rows) {
    while (nextLearning < learning.length && learning[nextLearning].target_at <= row.at) {
      const learned = learning[nextLearning++];
      const p = learned.predictions[fixedIndex];
      const y = learned.observed_delta;
      const q = confidence(p, y);
      A = params.lambda * A + q * p * y;
      B = params.lambda * B + q * p * p;
      if (q > 0) evidenceUpdates += 1;
      evidenceConfidence += q;
      evidenceInformation += q * p * p;
    }
    const unclamped = B > 0 ? A / B : 1;
    const rawFactor = clamp(unclamped, params.lo, params.hi);
    out.push({
      ...row,
      fixed: row.predictions[fixedIndex],
      fullPersonal: row.predictions[fixedIndex] === null
        ? null : row.predictions[fixedIndex] * rawFactor,
      rawFactor,
      unclampedFactor: unclamped,
      evidenceUpdates,
      evidenceConfidence,
      evidenceInformation,
    });
  }
  return out;
}
const baseRows = [];
for (const rows of grouped.values()) {
  const quota = rows[0]?.quota;
  if (!quota) continue;
  baseRows.push(...replayGroup(rows, learners[quota]));
}

function scope(quota, horizon, split) {
  return baseRows.filter(r => r.quota === quota && r.horizon === horizon
    && (split === 'all' || r.split === split));
}
function metric(rows, prediction) {
  const usable = rows
    .map(r => ({ row: r, p: prediction(r) }))
    .filter(x => x.p !== null);
  const errors = usable.map(x => Math.abs(x.p - x.row.observed_delta));
  const biases = usable.map(x => x.p - x.row.observed_delta);
  const sessions = new Map();
  for (const x of usable) {
    if (!sessions.has(x.row.session)) sessions.set(x.row.session, []);
    sessions.get(x.row.session).push(Math.abs(x.p - x.row.observed_delta));
  }
  return {
    count: usable.length,
    sessions: sessions.size,
    mae: mean(errors),
    median: quantile(errors, 0.5),
    p90: quantile(errors, 0.9),
    bias: mean(biases),
    equalSessionMae: mean([...sessions.values()].map(mean)),
  };
}
function compare(rows, prediction, baseline = r => r.fixed) {
  const usable = rows.filter(r => prediction(r) !== null && baseline(r) !== null);
  let wins = 0, losses = 0;
  for (const r of usable) {
    const e = Math.abs(prediction(r) - r.observed_delta);
    const b = Math.abs(baseline(r) - r.observed_delta);
    if (e < b - 1e-9) wins++;
    else if (e > b + 1e-9) losses++;
  }
  return { wins, losses, ties: usable.length - wins - losses };
}

const fixedMetrics = {};
for (const quota of ['5H','WK']) {
  fixedMetrics[quota] = metric(scope(quota, 300, 'earlier'), r => r.fixed);
}

function scoreTrust(quota, spec) {
  const rows = scope(quota, 300, 'earlier');
  const pred = r => predictionFor(r, spec);
  const m = metric(rows, pred);
  const fixed = fixedMetrics[quota];
  return {
    quota, key: spec.key, type: spec.type, K: spec.K, ...m,
    eligible: m.count === fixed.count
      && m.p90 <= fixed.p90 * 1.02 + 1e-12
      && m.mae <= fixed.mae * 1.02 + 1e-12,
  };
}
const candidateScores = [];
const selections = {};
for (const quota of ['5H','WK']) {
  const scored = trustGrid.map(spec => scoreTrust(quota, spec));
  candidateScores.push(...scored);
  const eligible = scored.filter(x => x.eligible).sort((a,b) =>
    a.mae - b.mae
    || a.equalSessionMae - b.equalSessionMae
    || a.p90 - b.p90
    || a.key.localeCompare(b.key));
  const winner = eligible[0] ?? null;
  selections[quota] = winner
    ? trustGrid.find(t => t.key === winner.key)
    : null;
}

const summary = [];
for (const quota of ['5H','WK']) {
  const selected = selections[quota];
  for (const horizon of [300,900,1800,3600]) {
    for (const split of ['earlier','holdout','all']) {
      const rows = scope(quota, horizon, split);
      const variantsToScore = [
        ['fixed', r => r.fixed],
        ['full_personal', r => r.fullPersonal],
      ];
      if (selected) variantsToScore.push(['evidence_weighted', r => predictionFor(r, selected)]);
      for (const [variant, pred] of variantsToScore) {
        const m = metric(rows, pred);
        const cmp = compare(rows, pred);
        const used = rows.filter(r => pred(r) !== null);
        const trusts = selected && variant === 'evidence_weighted'
          ? used.map(r => trustFor(r, selected)) : [];
        const factors = selected && variant === 'evidence_weighted'
          ? used.map(r => 1 + trustFor(r, selected) * (r.rawFactor - 1)) : [];
        summary.push({
          quota, horizonSeconds: horizon, split, variant, ...m, ...cmp,
          trustMean: mean(trusts),
          trustMin: trusts.length ? Math.min(...trusts) : null,
          trustMax: trusts.length ? Math.max(...trusts) : null,
          factorMean: mean(factors),
          factorMin: factors.length ? Math.min(...factors) : null,
          factorMax: factors.length ? Math.max(...factors) : null,
        });
      }
    }
  }
}
const candidateHoldout = [];
for (const quota of ['5H','WK']) {
  for (const spec of trustGrid) {
    for (const split of ['earlier','holdout','all']) {
      const rows = scope(quota, 300, split);
      const pred = r => predictionFor(r, spec);
      const m = metric(rows, pred);
      const cmp = compare(rows, pred);
      candidateHoldout.push({
        quota, split, key: spec.key, type: spec.type, K: spec.K,
        ...m, ...cmp,
        trustMean: mean(rows.map(r => trustFor(r, spec))),
      });
    }
  }
}

function bucket(n) {
  if (n === 0) return '0';
  if (n <= 2) return '1-2';
  if (n <= 4) return '3-4';
  if (n <= 6) return '5-6';
  if (n <= 9) return '7-9';
  return '10+';
}
const learningCurve = [];
for (const quota of ['5H','WK']) {
  const selected = selections[quota];
  const rows = scope(quota, 300, 'all');
  for (const b of ['0','1-2','3-4','5-6','7-9','10+']) {
    const subset = rows.filter(r => bucket(r.evidenceUpdates) === b);
    const fixed = metric(subset, r => r.fixed);
    const full = metric(subset, r => r.fullPersonal);
    const weighted = selected
      ? metric(subset, r => predictionFor(r, selected))
      : fixed;
    learningCurve.push({
      quota, bucket: b, count: fixed.count,
      fixedMae: fixed.mae,
      fullPersonalMae: full.mae,
      weightedMae: weighted.mae,
      trustMean: selected && subset.length
        ? mean(subset.map(r => trustFor(r, selected))) : null,
    });
  }
}
// Internal invariants: evidence is causal/monotone and trust only attenuates the raw factor.
for (const [key, rows] of grouped) {
  const replayed = baseRows
    .filter(r => `${r.session}|${r.quota}` === key)
    .sort((a,b) => a.at - b.at || a.horizon - b.horizon);
  let previous = -1;
  for (const row of replayed) {
    if (row.evidenceUpdates < previous)
      throw new Error(`non-monotone evidence in ${key}`);
    previous = row.evidenceUpdates;
  }
}
for (const quota of ['5H','WK']) {
  const selected = selections[quota];
  if (!selected) continue;
  for (const row of baseRows.filter(r => r.quota === quota && r.fixed !== null)) {
    const g = trustFor(row, selected);
    if (!(g >= 0 && g <= 1)) throw new Error(`invalid trust ${g}`);
    const ef = 1 + g * (row.rawFactor - 1);
    const lo = Math.min(1, row.rawFactor) - 1e-12;
    const hi = Math.max(1, row.rawFactor) + 1e-12;
    if (ef < lo || ef > hi) throw new Error(`non-attenuating factor ${ef}`);
  }
}

const evidenceDistribution = [];
for (const quota of ['5H','WK']) {
  const rows = scope(quota, 300, 'all');
  evidenceDistribution.push({
    quota,
    rows: rows.length,
    maxUpdates: rows.length ? Math.max(...rows.map(r => r.evidenceUpdates)) : null,
    meanUpdates: mean(rows.map(r => r.evidenceUpdates)),
    maxConfidence: rows.length ? Math.max(...rows.map(r => r.evidenceConfidence)) : null,
    meanConfidence: mean(rows.map(r => r.evidenceConfidence)),
    maxInformation: rows.length ? Math.max(...rows.map(r => r.evidenceInformation)) : null,
    meanInformation: mean(rows.map(r => r.evidenceInformation)),
  });
}

mkdirSync(output, { recursive: true });
const result = {
  metadata: data.metadata,
  splitAt: data.splitAt,
  fixedLearners: learners,
  trustGrid,
  selections,
  fixedMetrics,
  candidateScores,
  summary,
  candidateHoldout,
  learningCurve,
  evidenceDistribution,
};
writeFileSync(resolve(output, 'result.json'), JSON.stringify(result, null, 2) + '\n');
const quote = value => `"${String(value ?? '').replaceAll('"','""')}"`;
function csv(name, rows) {
  if (!rows.length) return;
  const columns = Object.keys(rows[0]);
  const body = [
    columns.map(quote).join(','),
    ...rows.map(r => columns.map(c => quote(r[c])).join(',')),
  ].join('\n') + '\n';
  writeFileSync(resolve(output, name), body);
}
csv('candidate-scores.csv', candidateScores);
csv('candidate-all-splits.csv', candidateHoldout);
csv('summary.csv', summary);
csv('learning-curve.csv', learningCurve);
csv('evidence-distribution.csv', evidenceDistribution);

console.log(JSON.stringify({ selections, evidenceDistribution }, null, 2));
for (const row of summary.filter(r =>
  r.horizonSeconds === 300 && ['earlier','holdout','all'].includes(r.split)))
  console.log(JSON.stringify(row));
console.log('learningCurve=' + JSON.stringify(learningCurve));
