import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs';
import { resolve, dirname } from 'node:path';

const input = resolve(process.argv[2] ?? 'output/burn-rate-replay/adaptive-comparison.json');
const output = resolve(process.argv[3] ?? 'output/burn-rate-replay/adaptive-summary');
const data = JSON.parse(readFileSync(input, 'utf8'));
mkdirSync(output, { recursive: true });
const fixtureKey = f => JSON.stringify(f);
function aggregate(rows, response) {
  const reference = new Map(rows.filter(r => r.variant === 'legacy').map(r => [fixtureKey(r.fixture), r.metrics]));
  const groups = new Map();
  for (const row of rows) {
    const f = row.fixture;
    const direction = response ? (f.new_period < f.period ? 'acceleration' : 'deceleration') : 'constant';
    const key = JSON.stringify([f.quota, f.cadence, f.fractional, direction, row.variant]);
    if (!groups.has(key)) groups.set(key, {
      quota: f.quota, cadenceSeconds: f.cadence, fractional: f.fractional, direction, variant: row.variant,
      fixtures: 0, available: 0, unavailable: 0, seconds: 0, errorSeconds: 0, signedErrorSeconds: 0,
      legacyErrorSeconds: 0, peakRelativeError: 0, legacyPeakRelativeError: 0, changes: 0,
      lowerErrorFixtures: 0, slowerSustainedFixtures: 0, missingSustainedFixtures: 0,
      maximumExcursion: 0, legacyMaximumExcursion: 0, worseExcursionFixtures: 0,
      worstErrorRatio: 0, worstFixture: '',
    });
    const g = groups.get(key), m = row.metrics, baseline = reference.get(fixtureKey(f));
    g.fixtures++;
    for (const [to, from] of [['available','available'],['unavailable','unavailable'],['seconds','seconds'],
      ['errorSeconds','error_seconds'],['signedErrorSeconds','signed_error_seconds'],['changes','changes']]) g[to] += m[from];
    g.legacyErrorSeconds += baseline.error_seconds;
    g.peakRelativeError = Math.max(g.peakRelativeError, m.peak_relative_error);
    g.legacyPeakRelativeError = Math.max(g.legacyPeakRelativeError, baseline.peak_relative_error);
    g.maximumExcursion = Math.max(g.maximumExcursion, m.excursion);
    g.legacyMaximumExcursion = Math.max(g.legacyMaximumExcursion, baseline.excursion);
    if (m.excursion > baseline.excursion + 1e-9) g.worseExcursionFixtures++;
    if (m.error_seconds < baseline.error_seconds - 1e-9) g.lowerErrorFixtures++;
    if (response && m.sustained_band === null) g.missingSustainedFixtures++;
    if (response && baseline.sustained_band !== null && (m.sustained_band === null || m.sustained_band > baseline.sustained_band)) g.slowerSustainedFixtures++;
    const ratio = baseline.error_seconds > 0 ? m.error_seconds / baseline.error_seconds : null;
    if (ratio !== null && ratio >= g.worstErrorRatio) { g.worstErrorRatio = ratio; g.worstFixture = fixtureKey(f); }
  }
  return [...groups.values()].map(g => ({ ...g,
    meanRelativeError: g.seconds > 0 ? g.errorSeconds / g.seconds : null,
    signedRelativeBias: g.seconds > 0 ? g.signedErrorSeconds / g.seconds : null,
    errorRatio: g.legacyErrorSeconds > 0 ? g.errorSeconds / g.legacyErrorSeconds : null,
  }));
}
function csv(name, rows) {
  const columns = Object.keys(rows[0]);
  const quote = value => `"${String(value ?? '').replaceAll('"', '""')}"`;
  writeFileSync(resolve(output, name), [columns.map(quote).join(','), ...rows.map(r => columns.map(c => quote(r[c])).join(','))].join('\n') + '\n');
}
const summary = { constant: aggregate(data.constant, false), response: aggregate(data.response, true),
  canonical: data.canonical.map(r => ({ ...r.fixture, variant: r.variant, ...r.metrics })) };
writeFileSync(resolve(output, 'summary.json'), JSON.stringify(summary, null, 2) + '\n');
csv('constant.csv', summary.constant);
csv('response.csv', summary.response);
csv('canonical.csv', summary.canonical);
const stressPath = resolve(dirname(input), 'adaptive-stress.json');
if (existsSync(stressPath)) csv('stress.csv', JSON.parse(readFileSync(stressPath, 'utf8')));
console.log(`Wrote ${summary.constant.length} constant groups, ${summary.response.length} response groups and ${summary.canonical.length} canonical rows to ${output}`);
