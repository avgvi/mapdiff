// elkjs on the calling thread. For tests and scripts; a page uses
// `engine-worker.ts`.

import ELK from 'elkjs/lib/elk.bundled.js'
import type { Engine } from './engine.ts'

export function inlineEngine(): Engine {
  const elk = new ELK()
  return { layout: (graph) => elk.layout(graph) }
}
