// Compares the regenerated replay evidence with the committed evidence.
// The gate is byte-identical, not "close enough".
import { readFileSync, existsSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(process.argv[2] ?? '.');
const pairs = [
  ['output/burn-rate-replay/real-summary/forecast.csv', 'docs/handoff/burn-rate-real-evidence/forecast.csv'],
  ['output/burn-rate-replay/real-summary/scenarios.csv', 'docs/handoff/burn-rate-real-evidence/scenarios.csv'],
  ['output/burn-rate-replay/evidence-weighted/candidate-all-splits.csv', 'docs/handoff/burn-rate-evidence-weighted-evidence/candidate-all-splits.csv'],
  ['output/burn-rate-replay/evidence-weighted/candidate-scores.csv', 'docs/handoff/burn-rate-evidence-weighted-evidence/candidate-scores.csv'],
  ['output/burn-rate-replay/evidence-weighted/evidence-distribution.csv', 'docs/handoff/burn-rate-evidence-weighted-evidence/evidence-distribution.csv'],
  ['output/burn-rate-replay/evidence-weighted/learning-curve.csv', 'docs/handoff/burn-rate-evidence-weighted-evidence/learning-curve.csv'],
  ['output/burn-rate-replay/evidence-weighted/summary.csv', 'docs/handoff/burn-rate-evidence-weighted-evidence/summary.csv'],
  ['output/burn-rate-replay/personal-calibration/activation-diagnostics.csv', 'docs/handoff/burn-rate-personal-calibration-evidence/activation-diagnostics.csv'],
  ['output/burn-rate-replay/personal-calibration/learning-curve.csv', 'docs/handoff/burn-rate-personal-calibration-evidence/learning-curve.csv'],
  ['output/burn-rate-replay/personal-calibration/summary.csv', 'docs/handoff/burn-rate-personal-calibration-evidence/summary.csv'],
  ['output/burn-rate-replay/personal-evidence-decay/real-summary.csv', 'docs/handoff/burn-rate-personal-evidence-decay-evidence/real-summary.csv'],
  ['output/burn-rate-replay/personal-evidence-decay/stress-checkpoints.csv', 'docs/handoff/burn-rate-personal-evidence-decay-evidence/stress-checkpoints.csv'],
  ['output/burn-rate-replay/personal-evidence-decay/stress-summary.csv', 'docs/handoff/burn-rate-personal-evidence-decay-evidence/stress-summary.csv'],
];

let failures = 0;
for (const [fresh, committed] of pairs) {
  const a = resolve(root, fresh);
  const b = resolve(root, committed);
  if (!existsSync(a)) { console.log(`MISSING ${fresh}`); failures++; continue; }
  if (!existsSync(b)) { console.log(`MISSING ${committed}`); failures++; continue; }
  const left = readFileSync(a);
  const right = readFileSync(b);
  if (left.equals(right)) {
    console.log(`OK   ${fresh}`);
  } else {
    failures++;
    console.log(`DIFF ${fresh} (${left.length} vs ${right.length} bytes)`);
    const leftText = left.toString('utf8').split(/\r?\n/);
    const rightText = right.toString('utf8').split(/\r?\n/);
    const shown = new Set();
    for (let i = 0; i < Math.max(leftText.length, rightText.length); i++) {
      if (leftText[i] !== rightText[i]) {
        if (!shown.has(i)) {
          shown.add(i);
          console.log(`  line ${i + 1}\n    fresh:    ${String(leftText[i]).slice(0, 160)}\n    committed: ${String(rightText[i]).slice(0, 160)}`);
        }
        if (shown.size >= 5) break;
      }
    }
  }
}
console.log(failures === 0 ? `ALL ${pairs.length} FILES BYTE-IDENTICAL` : `${failures} FILES DIFFER`);
process.exit(failures === 0 ? 0 : 1);
