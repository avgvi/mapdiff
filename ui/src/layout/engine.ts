// The layout engine: elkjs. It is passed in, so that the browser can run
// it in a web worker and tests can run it in place.

import type { ElkNode } from 'elkjs/lib/elk-api.js'

export type { ElkNode }

export interface Engine {
  layout(graph: ElkNode): Promise<ElkNode>
}
