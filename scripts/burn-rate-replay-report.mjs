// Summarize synthetic Rust replay evidence; no application state is read.
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const input = resolve(process.argv[2] ?? 'output/burn-rate-replay/parameter-grid.json');
const output = resolve(process.argv[3] ?? 'output/burn-rate-replay/parameter-grid.csv');
const grid = JSON.parse(readFileSync(input, 'utf8'));
const rows = grid.map(row => {
  const values = {
    quota: row.quota,
    halfLifeSeconds: row.halfLifeSeconds,
    localWindowSeconds: row.localWindowSeconds,
    worstConstantRelativeError: row.worstConstantRelativeError,
    worstLegacyConstantRelativeError: row.worstLegacyConstantRelativeError,
    candidateConstantRelativeErrorSeconds: row.candidateConstantRelativeErrorSeconds,
    legacyConstantRelativeErrorSeconds: row.legacyConstantRelativeErrorSeconds,
    constantPairedSeconds: row.constantPairedSeconds,
    constantAvailable: row.constantAvailable,
    constantUnavailable: row.constantUnavailable,
    worstConstantFixture: JSON.stringify(row.worstConstantFixture),
    responsePass: row.responsePass,
    pass: row.pass,
  };
  for (const direction of ['acceleration', 'deceleration']) {
    for (const [metric, value] of Object.entries(row[direction])) {
      values[`${direction}_${metric}`] = value;
    }
  }
  return values;
});
if (rows.length === 0) throw new Error('No parameter evidence');
const columns = Object.keys(rows[0]);
const quote = value => `"${String(value ?? '').replaceAll('"', '""')}"`;
const csv = [columns.map(quote).join(','), ...rows.map(row => columns.map(key => quote(row[key])).join(','))].join('\n') + '\n';
writeFileSync(output, csv, 'utf8');
console.log(`${grid.length} candidates; ${grid.filter(row => row.pass).length} pass. Summary: ${output}`);
