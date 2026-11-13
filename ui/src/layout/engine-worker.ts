// elkjs in a web worker, so that laying out a large map never blocks the
// page.

import ELK from 'elkjs/lib/elk-api.js'
import type { Engine } from './engine.ts'

export function workerEngine(): Engine & { stop(): void } {
  const elk = new ELK({
    workerFactory: () =>
      new Worker(new URL('elkjs/lib/elk-worker.min.js', import.meta.url), { type: 'classic' }),
  })
  return {
    layout: (graph) => elk.layout(graph),
    stop: () => elk.terminateWorker(),
  }
}
