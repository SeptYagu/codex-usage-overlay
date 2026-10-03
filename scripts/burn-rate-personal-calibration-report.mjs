import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';

const input = resolve(process.argv[2] ?? 'output/burn-rate-replay/real-forecasts.json');
const output = resolve(process.argv[3] ?? 'output/burn-rate-replay/personal-calibration');
const data = JSON.parse(readFileSync(input, 'utf8'));
const variants = data.variants.map(v => v.name);
const fixedIndex = variants.indexOf('fixed');
if (fixedIndex < 0) throw new Error('fixed variant not found');

const lambdas = [0.80, 0.90, 0.95, 0.98, 0.99];
const priors = [0.5, 2, 5, 10];
const bounds = [[0.5, 2.0], [0.75, 1.5], [0.8, 1.25]];
const policies = ['all', 'downweight_low', 'ignore_zero_near'];
const clamp = (x, lo, hi) => Math.max(lo, Math.min(hi, x));
const mean = xs => xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null;
const quantile = (xs, p) => xs.length
  ? [...xs].sort((a, b) => a - b)[Math.ceil(xs.length * p) - 1] : null;
function confidence(policy, predicted, observed) {
  if (policy === 'all') return 1;
  if (policy === 'ignore_zero_near')
    return observed === 0 && predicted < 0.75 ? 0 : 1;
  const info = Math.max(predicted, observed);
  if (info < 0.5) return 0.1;
  if (info < 1) return 0.25;
  if (info < 2) return 0.5;
  return 1;
}

function keyOf(p) {
  return [
    `lambda=${p.lambda.toFixed(2)}`,
    `prior=${p.prior}`,
    `bounds=${p.lo}-${p.hi}`,
    `policy=${p.policy}`,
  ].join(';');
}

const parameterGrid = [];
for (const lambda of lambdas)
  for (const prior of priors)
    for (const [lo, hi] of bounds)
      for (const policy of policies)
        parameterGrid.push({ lambda, prior, lo, hi, policy, key: '' });
for (const p of parameterGrid) p.key = keyOf(p);
const grouped = new Map();
for (const row of data.forecasts) {
  const key = `${row.session}|${row.quota}`;
  if (!grouped.has(key)) grouped.set(key, []);
  grouped.get(key).push(row);
}
for (const rows of grouped.values())
  rows.sort((a, b) => a.at - b.at || a.horizon - b.horizon || a.target_at - b.target_at);

function replay(params) {
  const result = [];
  for (const rows of grouped.values()) {
    const learning = rows
      .filter(r => r.horizon === 300 && r.predictions[fixedIndex] !== null)
      .sort((a, b) => a.target_at - b.target_at || a.at - b.at);
    let nextLearning = 0;
    let A = params.prior, B = params.prior, updates = 0;
    for (const row of rows) {
      while (nextLearning < learning.length && learning[nextLearning].target_at <= row.at) {
        const learned = learning[nextLearning++];
        const p = learned.predictions[fixedIndex];
        const y = learned.observed_delta;
        const q = confidence(params.policy, p, y);
        A = params.lambda * A + q * p * y;
        B = params.lambda * B + q * p * p;
        if (q > 0) updates++;
      }
      const rawFactor = B > 0 ? A / B : 1;
      const factor = clamp(rawFactor, params.lo, params.hi);
      const fixed = row.predictions[fixedIndex];
      result.push({
        session: row.session,
        quota: row.quota,
        horizon: row.horizon,
        at: row.at,
        target_at: row.target_at,
        split: row.split,
        observed_delta: row.observed_delta,
        fixed,
        calibrated: fixed === null ? null : fixed * factor,
        factor,
        rawFactor,
        updates,
        boundHit: rawFactor < params.lo || rawFactor > params.hi,
      });
    }
  }
  return result;
}

function metric(rows, field) {
  const usable = rows.filter(r => r[field] !== null);
  const errors = usable.map(r => Math.abs(r[field] - r.observed_delta));
  const bias = usable.map(r => r[field] - r.observed_delta);
  const bySession = new Map();
  for (const row of usable) {
    if (!bySession.has(row.session)) bySession.set(row.session, []);
    bySession.get(row.session).push(Math.abs(row[field] - row.observed_delta));
  }
  return {
    count: usable.length,
    sessions: bySession.size,
    mae: mean(errors),
    median: quantile(errors, 0.5),
    p90: quantile(errors, 0.9),
    bias: mean(bias),
    equalSessionMae: mean([...bySession.values()].map(mean)),
  };
}

function scope(rows, quota, horizon, split) {
  return rows.filter(r => r.quota === quota && r.horizon === horizon
    && (split === 'all' || r.split === split));
}

const fixedRows = replay({
  lambda: 1, prior: 1, lo: 1, hi: 1, policy: 'all', key: 'fixed',
});
const fixedBaseline = {};
for (const quota of ['5H', 'WK']) {
  fixedBaseline[quota] = {};
  for (const split of ['earlier', 'holdout', 'all'])
    fixedBaseline[quota][split] = metric(scope(fixedRows, quota, 300, split), 'fixed');
}
function scoreCandidate(params, quota) {
  const rows = replay(params);
  const earlier = scope(rows, quota, 300, 'earlier');
  const m = metric(earlier, 'calibrated');
  const boundHits = earlier.filter(r => r.fixed !== null && r.boundHit).length;
  const fixed = fixedBaseline[quota].earlier;
  return {
    params,
    ...m,
    boundHits,
    boundHitRate: m.count ? boundHits / m.count : 0,
    eligible: m.count > 0
      && m.p90 <= fixed.p90 * 1.10 + 1e-12
      && boundHits / m.count <= 0.10 + 1e-12,
  };
}

function choose(quota) {
  const scored = parameterGrid.map(p => scoreCandidate(p, quota));
  const eligible = scored.filter(x => x.eligible);
  eligible.sort((a, b) =>
    a.mae - b.mae
    || a.equalSessionMae - b.equalSessionMae
    || a.p90 - b.p90
    || a.params.key.localeCompare(b.params.key));
  return { selected: eligible[0] ?? null, scored };
}

const choices = { '5H': choose('5H'), WK: choose('WK') };
const selectedRows = {};
for (const quota of ['5H', 'WK'])
  selectedRows[quota] = choices[quota].selected
    ? replay(choices[quota].selected.params)
    : fixedRows;

const summary = [];
for (const quota of ['5H', 'WK']) {
  for (const horizon of [300, 900, 1800, 3600]) {
    for (const split of ['earlier', 'holdout', 'all']) {
      const rows = scope(selectedRows[quota], quota, horizon, split);
      const fixed = metric(rows, 'fixed');
      const calibrated = metric(rows, 'calibrated');
      const comparable = rows.filter(r => r.fixed !== null && r.calibrated !== null);
      const wins = comparable.filter(r =>
        Math.abs(r.calibrated - r.observed_delta)
          < Math.abs(r.fixed - r.observed_delta) - 1e-9).length;
      const losses = comparable.filter(r =>
        Math.abs(r.calibrated - r.observed_delta)
          > Math.abs(r.fixed - r.observed_delta) + 1e-9).length;
      const hits = comparable.filter(r => r.boundHit).length;
      summary.push({
        quota, horizonSeconds: horizon, split,
        ...Object.fromEntries(Object.entries(fixed).map(([k, v]) => [`fixed_${k}`, v])),
        ...Object.fromEntries(Object.entries(calibrated).map(([k, v]) => [`cal_${k}`, v])),
        wins, losses, ties: comparable.length - wins - losses,
        boundHits: hits,
        factorMean: mean(comparable.map(r => r.factor)),
        factorMin: comparable.length ? Math.min(...comparable.map(r => r.factor)) : null,
        factorMax: comparable.length ? Math.max(...comparable.map(r => r.factor)) : null,
      });
    }
  }
}
function learningBucket(n) {
  if (n === 0) return 'cold_start';
  if (n <= 4) return '1-4';
  if (n <= 9) return '5-9';
  if (n <= 19) return '10-19';
  if (n <= 49) return '20-49';
  return '50+';
}

const learningCurve = [];
for (const quota of ['5H', 'WK']) {
  const rows = scope(selectedRows[quota], quota, 300, 'all')
    .filter(r => r.fixed !== null && r.calibrated !== null);
  for (const bucket of ['cold_start', '1-4', '5-9', '10-19', '20-49', '50+']) {
    const b = rows.filter(r => learningBucket(r.updates) === bucket);
    if (!b.length) {
      learningCurve.push({ quota, bucket, count: 0, fixedMae: null, calibratedMae: null });
      continue;
    }
    learningCurve.push({
      quota, bucket, count: b.length,
      fixedMae: mean(b.map(r => Math.abs(r.fixed - r.observed_delta))),
      calibratedMae: mean(b.map(r => Math.abs(r.calibrated - r.observed_delta))),
      factorMean: mean(b.map(r => r.factor)),
    });
  }
}

const activationDiagnostics = [];
for (const quota of ['5H', 'WK']) {
  for (const minUpdates of [0, 3, 5, 8]) {
    for (const split of ['earlier', 'holdout', 'all']) {
      const rows = scope(selectedRows[quota], quota, 300, split)
        .map(r => ({ ...r, gated: r.updates >= minUpdates ? r.calibrated : r.fixed }));
      const fixed = metric(rows, 'fixed');
      const gated = metric(rows, 'gated');
      activationDiagnostics.push({
        quota, split, minUpdates,
        fixedMae: fixed.mae, gatedMae: gated.mae,
        fixedP90: fixed.p90, gatedP90: gated.p90,
        fixedEqualSessionMae: fixed.equalSessionMae,
        gatedEqualSessionMae: gated.equalSessionMae,
        active: rows.filter(r => r.fixed !== null && r.updates >= minUpdates).length,
      });
    }
  }
}
mkdirSync(output, { recursive: true });
const result = {
  metadata: data.metadata,
  splitAt: data.splitAt,
  fixedIndex,
  gridSize: parameterGrid.length,
  choices: Object.fromEntries(['5H', 'WK'].map(q => [q, {
    selected: choices[q].selected,
    topTen: choices[q].scored
      .filter(x => x.eligible)
      .sort((a, b) => a.mae - b.mae || a.equalSessionMae - b.equalSessionMae)
      .slice(0, 10),
  }])),
  summary,
  learningCurve,
  activationDiagnostics,
};
writeFileSync(resolve(output, 'result.json'), JSON.stringify(result, null, 2) + '\n');

const quote = v => `"${String(v ?? '').replaceAll('"', '""')}"`;
function csv(name, rows) {
  if (!rows.length) return;
  const cols = Object.keys(rows[0]);
  const body = [cols.map(quote).join(','),
    ...rows.map(r => cols.map(c => quote(r[c])).join(','))].join('\n') + '\n';
  writeFileSync(resolve(output, name), body);
}
csv('summary.csv', summary);
csv('learning-curve.csv', learningCurve);
csv('activation-diagnostics.csv', activationDiagnostics);
const compactChoice = quota => {
  const s = choices[quota].selected;
  return s ? {
    key: s.params.key,
    earlierMae: s.mae,
    earlierP90: s.p90,
    earlierEqualSessionMae: s.equalSessionMae,
    boundHitRate: s.boundHitRate,
    effectiveUpdates: 1 / (1 - s.params.lambda),
  } : null;
};
console.log(JSON.stringify({
  gridSize: parameterGrid.length,
  selected5H: compactChoice('5H'),
  selectedWK: compactChoice('WK'),
}, null, 2));
for (const row of summary.filter(r =>
  r.horizonSeconds === 300 && ['earlier', 'holdout', 'all'].includes(r.split)))
  console.log(JSON.stringify(row));
console.log('learningCurve=' + JSON.stringify(learningCurve));
