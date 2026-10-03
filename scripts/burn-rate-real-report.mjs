import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';

const input = resolve(process.argv[2] ?? 'output/burn-rate-replay/real-forecasts.json');
const output = resolve(process.argv[3] ?? 'output/burn-rate-replay/real-summary');
const data = JSON.parse(readFileSync(input, 'utf8'));
const variants = data.variants.map(v => v.name);
const legacyIndex = variants.indexOf('legacy'), fixedIndex = variants.indexOf('fixed');
const mean = values => values.length ? values.reduce((a, b) => a + b, 0) / values.length : null;
const quantile = (values, p) => values.length ? [...values].sort((a, b) => a - b)[Math.ceil(values.length * p) - 1] : null;
const summary = [];
for (const quota of ['5H', 'WK']) {
  for (const horizon of [300, 900, 1800, 3600]) {
    for (const split of ['earlier', 'holdout', 'all']) {
      const pool = data.forecasts.filter(r => r.quota === quota && r.horizon === horizon
        && (split === 'all' || r.split === split));
      // Identical outcome windows across every estimator; missing forecasts are counted separately.
      const common = pool.filter(r => r.predictions.every(p => p !== null));
      for (let index = 0; index <= variants.length; index++) {
        const variant = variants[index] ?? 'zero_increment';
        const prediction = row => index === variants.length ? 0 : row.predictions[index];
        const errors = common.map(r => Math.abs(prediction(r) - r.observed_delta));
        const bias = common.map(r => prediction(r) - r.observed_delta);
        const sessions = new Map();
        for (const row of common) {
          if (!sessions.has(row.session)) sessions.set(row.session, []);
          sessions.get(row.session).push(row);
        }
        const sessionMeans = [...sessions.values()].map(rows =>
          mean(rows.map(r => Math.abs(prediction(r) - r.observed_delta))));
        const wins = baselineIndex => common.filter(r => Math.abs(prediction(r) - r.observed_delta)
          < Math.abs(r.predictions[baselineIndex] - r.observed_delta) - 1e-9).length;
        const losses = baselineIndex => common.filter(r => Math.abs(prediction(r) - r.observed_delta)
          > Math.abs(r.predictions[baselineIndex] - r.observed_delta) + 1e-9).length;
        summary.push({ quota, horizonSeconds: horizon, split, variant, eligible: pool.length,
          available: pool.filter(r => prediction(r) !== null).length, common: common.length,
          sessions: sessions.size,
          maePoints: mean(errors), medianPoints: quantile(errors, .5), p90Points: quantile(errors, .9),
          equalSessionMaePoints: mean(sessionMeans),
          excessBeyondOnePoint: mean(errors.map(e => Math.max(0, e - 1))), biasPoints: mean(bias),
          observedZero: common.filter(r => r.observed_delta === 0).length,
          winsLegacy: wins(legacyIndex), lossesLegacy: losses(legacyIndex),
          winsFixed: wins(fixedIndex), lossesFixed: losses(fixedIndex) });
      }
    }
  }
}
const scenarios = [];
for (const quota of ['5H', 'WK']) {
  for (const horizon of [300, 900, 1800, 3600]) {
    for (const split of ['earlier', 'holdout', 'all']) {
      for (const scenario of ['zero', 'one_point', 'two_plus']) {
        const rows = data.forecasts.filter(r => r.quota === quota && r.horizon === horizon
          && (split === 'all' || r.split === split) && r.predictions.every(p => p !== null)
          && (scenario === 'zero' ? r.observed_delta === 0 : scenario === 'one_point'
            ? r.observed_delta === 1 : r.observed_delta >= 2));
        for (let i = 0; i < variants.length; i++) scenarios.push({ quota, horizonSeconds: horizon,
          split, scenario, variant: variants[i], common: rows.length,
          maePoints: mean(rows.map(r => Math.abs(r.predictions[i] - r.observed_delta))) });
      }
    }
  }
}
mkdirSync(output, { recursive: true });
const quote = value => `"${String(value ?? '').replaceAll('"', '""')}"`;
function csv(name, rows) {
  const columns = Object.keys(rows[0]);
  writeFileSync(resolve(output, name), [columns.map(quote).join(','),
    ...rows.map(r => columns.map(c => quote(r[c])).join(','))].join('\n') + '\n');
}
writeFileSync(resolve(output, 'summary.json'), JSON.stringify({ metadata: data.metadata,
  splitAt: data.splitAt, summary, scenarios }, null, 2) + '\n');
csv('forecast.csv', summary);
csv('scenarios.csv', scenarios);
console.log(JSON.stringify({ metadata: data.metadata, intervals: data.forecasts.length, groups: summary.length }));
for (const row of summary.filter(r => ['legacy', 'fixed', 'adaptive_only', 'zero_increment'].includes(r.variant)
  && r.horizonSeconds === 300 && ['all', 'holdout'].includes(r.split))) console.log(JSON.stringify(row));
