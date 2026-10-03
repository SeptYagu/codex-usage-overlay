// Extract only quota observations; never copy conversation text or account/credit data.
import { readFileSync, readdirSync, mkdirSync, writeFileSync } from 'node:fs';
import { resolve, join, dirname } from 'node:path';
import { homedir } from 'node:os';

const root = resolve(process.argv[2] ?? join(homedir(), '.codex', 'sessions'));
const output = resolve(process.argv[3] ?? 'output/burn-rate-replay/real-input.json');
const start = process.argv[4] ?? '2026-09-27';
const end = process.argv[5] ?? '2026-10-03';
const sessions = [];
const counts = { files: 0, records: 0, invalid: 0, conflictingSeconds: 0, duplicateSeconds: 0 };
const valid = q => q && Number.isFinite(q.used_percent) && q.used_percent >= 0
  && q.used_percent <= 100 && Number.isSafeInteger(q.resets_at) && q.resets_at > 0;
for (const year of readdirSync(root).filter(s => /^\d{4}$/.test(s)).sort()) {
  for (const month of readdirSync(join(root, year)).filter(s => /^\d{2}$/.test(s)).sort()) {
    for (const day of readdirSync(join(root, year, month)).filter(s => /^\d{2}$/.test(s)).sort()) {
      const date = `${year}-${month}-${day}`;
      if (date < start || date > end) continue;
      const dir = join(root, year, month, day);
      for (const name of readdirSync(dir).filter(s => s.endsWith('.jsonl')).sort()) {
        counts.files++;
        const bySecond = new Map(), conflicts = new Set();
        for (const line of readFileSync(join(dir, name), 'utf8').split('\n')) {
          if (!line.includes('"token_count"')) continue;
          let row;
          try { row = JSON.parse(line); } catch { continue; }
          const limits = row.payload?.rate_limits;
          if (row.payload?.type !== 'token_count' || limits?.limit_id !== 'codex'
            || limits.primary?.window_minutes !== 300 || limits.secondary?.window_minutes !== 10080) continue;
          if (typeof row.timestamp !== 'string') { counts.invalid++; continue; }
          const at = Math.floor(Date.parse(row.timestamp) / 1000);
          const recordDate = row.timestamp.slice(0, 10);
          if (recordDate < start || recordDate > end) continue;
          if (!Number.isSafeInteger(at) || !valid(limits.primary) || !valid(limits.secondary)) {
            counts.invalid++; continue;
          }
          counts.records++;
          const obs = { at, five: limits.primary.used_percent, weekly: limits.secondary.used_percent,
            reset: limits.primary.resets_at, weekReset: limits.secondary.resets_at };
          if (bySecond.has(at)) {
            if (JSON.stringify(bySecond.get(at)) !== JSON.stringify(obs)) conflicts.add(at);
            else counts.duplicateSeconds++;
          } else bySecond.set(at, obs);
        }
        counts.conflictingSeconds += conflicts.size;
        const observations = [...bySecond.values()].filter(r => !conflicts.has(r.at)).sort((a, b) => a.at - b.at);
        if (observations.length) sessions.push({ id: sessions.length, observations });
      }
    }
  }
}
const all = sessions.flatMap(s => s.observations);
if (!all.length) throw new Error('No valid quota observations in selected range');
const metadata = { start, end, ...counts, sessions: sessions.length, observations: all.length,
  firstAt: Math.min(...all.map(r => r.at)), lastAt: Math.max(...all.map(r => r.at)),
  fractionalFive: all.filter(r => !Number.isInteger(r.five)).length,
  fractionalWeekly: all.filter(r => !Number.isInteger(r.weekly)).length };
mkdirSync(dirname(output), { recursive: true });
writeFileSync(output, JSON.stringify({ metadata, sessions }) + '\n');
console.log(JSON.stringify({ output, metadata }));
