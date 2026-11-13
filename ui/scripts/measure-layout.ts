// Lays out maps written by `mapdiff map --json` and reports how long it took
// and whether the result is sound.
//
//   mapdiff map --json --level module --root some/repo > map.json
//   node scripts/measure-layout.ts map.json [more.json ...]

import { readFileSync } from 'node:fs'
import { boxProblems, routeProblems } from '../src/layout/check.ts'
import { inlineEngine } from '../src/layout/engine-inline.ts'
import { LayoutCache, layoutMap } from '../src/layout/layout.ts'
import type { MapData } from '../src/layout/types.ts'

const engine = inlineEngine()
let failed = false
for (const file of process.argv.slice(2)) {
  const map = JSON.parse(readFileSync(file, 'utf8')) as MapData
  const cache = new LayoutCache()
  const first = await layoutMap(map, engine, cache)
  // A second, fresh run: the engine is warm, and the answer must not change.
  const second = await layoutMap(map, engine, new LayoutCache())
  const again = await layoutMap(map, engine, cache)
  const same =
    JSON.stringify({ ...first, stats: 0 }) === JSON.stringify({ ...second, stats: 0 })
  const problems = [...boxProblems(first), ...routeProblems(first)]
  const routes = Object.values(first.containers).reduce(
    (sum, container) =>
      sum +
      container.wires.filter((wire) => wire.route).length +
      Object.keys(container.entries).length +
      Object.keys(container.exits).length,
    0,
  )
  console.log(
    [
      file.split('/').pop(),
      `${map.level}`,
      `${map.nodes.length} boxes`,
      `${first.stats.containers} containers`,
      `${routes} routes`,
      `${Math.round(first.width)} x ${Math.round(first.height)}`,
      `cold ${Math.round(first.stats.milliseconds)} ms`,
      `warm ${Math.round(second.stats.milliseconds)} ms`,
      `cached ${Math.round(again.stats.milliseconds)} ms (${again.stats.laidOut} laid out)`,
      `${first.stats.engineCalls} engine calls`,
      same ? 'same twice' : 'DIFFERS between runs',
      `${problems.length} problems`,
    ].join(' | '),
  )
  for (const problem of problems.slice(0, 8)) console.log(`  ${problem}`)
  if (problems.length > 0 || !same) failed = true
}
process.exit(failed ? 1 : 0)
