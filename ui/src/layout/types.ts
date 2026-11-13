// What the server sends for a map (`/api/map`, `mapdiff map --json`), and what
// a layout of it looks like. The first half mirrors
// `crates/mapdiff-graph/src/map.rs`.

export type Level = 'architecture' | 'package' | 'module' | 'code'
export type Presence = 'both' | 'added' | 'removed'
export type WireKind = 'direct' | 'cycle' | 'bridge'

export interface MapNode {
  id: string
  parent: string | null
  kind: string
  lang_kind: string
  label: string
  /** The line the scope starts on, counted from 1. 0 for a directory. */
  line: number
  lines: number
  columns: number
  /** Part of its parent's box at this level; drawn as a small mark. */
  minor: boolean
  presence: Presence
}

export interface MapWire {
  from: string
  to: string
  kind: WireKind
  count: number
  presence: Presence
}

export interface MapContainer {
  wires: MapWire[]
  entries: { id: string }[]
  exits: { id: string }[]
}

/** Why an unchanged box is affected by a change. */
export interface Cause {
  /** The box of the changed scope it uses. */
  at: string
  label: string
  reasons: string[]
}

/** A change, laid over its map. Positions are into the change set's list. */
export interface Overlay {
  /** For each box, the changes drawn in it. */
  changes: Record<string, number[]>
  /** Unchanged boxes that use, or are used by, something that changed. */
  near: string[]
  /** Unchanged boxes that use something whose change they can see. */
  impacted: Record<string, Cause[]>
  /** The changes in an order to read them in: what is used first; code, then tests, then prose. */
  order: number[]
  /** The changes that are to tests. Tests are not on the map: these are only listed. */
  tests?: number[]
}

export interface MapData {
  level: Level
  /** A parent before its children, siblings in source order. */
  nodes: MapNode[]
  containers: Record<string, MapContainer>
  /** Set on the map of a comparison. */
  overlay?: Overlay | null
}

/** How a box is drawn, which decides its size. */
export type Detail =
  /** A box around its children. */
  | 'container'
  /** A title bar and nothing else. */
  | 'title'
  /** One line in a column: a field, a variant, an import. */
  | 'row'
  /** A title bar over the scope's source text. */
  | 'code'
  /** A small mark with no text: a file, at the level of directories. */
  | 'pill'

export interface Point {
  x: number
  y: number
}

export interface Box {
  id: string
  parent: string | null
  /** The top left corner, measured from the parent's top left corner. */
  x: number
  y: number
  width: number
  height: number
  detail: Detail
  presence: Presence
}

export interface WireRoute {
  from: string
  to: string
  kind: WireKind
  count: number
  presence: Presence
  /**
   * From `from` to `to`, in the container's coordinates. A wire that
   * closes a loop runs against the flow: it leaves `from` on the left and
   * arrives at `to` on the right. Null for a wire to or from a row, which
   * a viewer draws straight.
   */
  route: Point[] | null
}

export interface ContainerLayout {
  wires: WireRoute[]
  /** From the container's way in (left border) to each child used from outside. */
  entries: Record<string, Point[]>
  /** From each child that uses the outside to the container's way out (right border). */
  exits: Record<string, Point[]>
}

export interface LayoutStats {
  /** Containers on the map. */
  containers: number
  /** Containers that had to be laid out; the rest came from the cache. */
  laidOut: number
  /** Calls to the layout engine. */
  engineCalls: number
  milliseconds: number
}

export interface MapLayout {
  root: string
  width: number
  height: number
  boxes: Record<string, Box>
  containers: Record<string, ContainerLayout>
  stats: LayoutStats
}
