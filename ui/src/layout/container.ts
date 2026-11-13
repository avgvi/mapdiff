// The layout of one container: where its children go, how the wires among
// them run, and how a wire from outside reaches a child.
//
// Children are stacked in up to three blocks, top to bottom:
//
//   1. rows (fields, variants, imports), in one column;
//   2. children joined by wires, laid out left to right by the engine so
//      that a wire runs from what uses to what is used;
//   3. the rest, in rows that wrap, in source order;
//   4. small marks (files, at the level of directories), packed tight.
//
// A container is entered on its left border and left on its right border,
// at title height. From there a wire runs down a lane inside the border
// and along the gap above its row, so that it never crosses a box.

import type { ElkNode } from './engine.ts'
import { SIZE, portOffset } from './sizes.ts'
import type { Point } from './types.ts'

export interface Child {
  id: string
  width: number
  height: number
  row: boolean
  /** A small mark: packed tight at the bottom, with no wires. */
  pill?: boolean
}

export interface WireInput {
  from: string
  to: string
  /** True for a wire that closes a loop: the layout flows the other way. */
  against: boolean
}

export interface ContainerInput {
  /** The width the container's own title needs. */
  titleWidth: number
  /** In the order they are shown. */
  children: Child[]
  wires: WireInput[]
  /** Children used from outside. */
  entries: string[]
  /** Children that use the outside. */
  exits: string[]
}

export interface ContainerResult {
  width: number
  height: number
  /** Top left corners, measured from the container's top left corner. */
  positions: Record<string, Point>
  /** The width every row is drawn at: that of the widest. */
  rowWidth: number
  /** One per wire of the input, in the same order. */
  wires: (Point[] | null)[]
  entries: Record<string, Point[]>
  exits: Record<string, Point[]>
}

/** What is known about a container before the engine has run. */
export interface Plan {
  input: ContainerInput
  rows: Child[]
  flow: Child[]
  rest: Child[]
  pills: Child[]
  /** The graph for the engine, when there is anything to flow. */
  graph: ElkNode | null
}

/** Everything a layout depends on, as text: equal keys give equal layouts. */
export function keyOf(input: ContainerInput): string {
  return JSON.stringify([
    input.titleWidth,
    input.children.map((child) => [
      child.id,
      child.width,
      child.height,
      child.row ? 1 : child.pill ? 2 : 0,
    ]),
    input.wires.map((wire) => [wire.from, wire.to, wire.against ? 1 : 0]),
    input.entries,
    input.exits,
  ])
}

const FLOW_OPTIONS = {
  'elk.algorithm': 'layered',
  'elk.direction': 'RIGHT',
  'elk.edgeRouting': 'ORTHOGONAL',
  // The container's own way in and way out sit on its left and right.
  'elk.portConstraints': 'FIXED_SIDE',
  // Keep source order wherever the wires leave a choice, so that a small
  // change to the code is a small change to the picture.
  'elk.layered.considerModelOrder.strategy': 'NODES_AND_EDGES',
  'elk.randomSeed': '1',
  'elk.padding': '[top=0,left=0,bottom=0,right=0]',
  'elk.spacing.nodeNode': String(SIZE.gap),
  'elk.spacing.edgeNode': String(SIZE.gap / 2),
  'elk.spacing.edgeEdge': String(SIZE.gap / 2),
  'elk.spacing.componentComponent': String(SIZE.gap),
  'elk.layered.spacing.nodeNodeBetweenLayers': String(SIZE.layerGap),
  'elk.layered.spacing.edgeNodeBetweenLayers': String(SIZE.gap / 2),
  'elk.layered.spacing.edgeEdgeBetweenLayers': String(SIZE.gap / 2),
  'elk.aspectRatio': '1.6',
}

/**
 * Sorts the children into blocks and builds the engine's graph. `name` is
 * a prefix that keeps this container's IDs apart from others laid out in
 * the same call.
 */
export function plan(input: ContainerInput, name: string): Plan {
  const wired = new Set<string>()
  const isRow = new Map(input.children.map((child) => [child.id, child.row]))
  for (const wire of input.wires) {
    if (isRow.get(wire.from) || isRow.get(wire.to)) continue
    wired.add(wire.from)
    wired.add(wire.to)
  }
  const rows = input.children.filter((child) => child.row)
  const flow = input.children.filter((child) => !child.row && !child.pill && wired.has(child.id))
  const pills = input.children.filter((child) => child.pill)
  const rest = input.children.filter(
    (child) => !child.row && !child.pill && !wired.has(child.id),
  )
  if (flow.length === 0) return { input, rows, flow, rest, pills, graph: null }

  const index = new Map(flow.map((child, position) => [child.id, position]))
  const node = (id: string) => `${name}/${index.get(id)}`
  const offset = portOffset(false)
  const graph: ElkNode = {
    id: name,
    layoutOptions: FLOW_OPTIONS,
    ports: [
      { id: `${name}:in`, width: 0, height: 0, layoutOptions: { 'elk.port.side': 'WEST' } },
      { id: `${name}:out`, width: 0, height: 0, layoutOptions: { 'elk.port.side': 'EAST' } },
    ],
    children: flow.map((child) => ({
      id: node(child.id),
      width: child.width,
      height: child.height,
      layoutOptions: { 'elk.portConstraints': 'FIXED_POS' },
      ports: [
        { id: `${node(child.id)}:in`, x: 0, y: offset, width: 0, height: 0 },
        { id: `${node(child.id)}:out`, x: child.width, y: offset, width: 0, height: 0 },
      ],
    })),
    edges: [],
  }
  input.wires.forEach((wire, position) => {
    if (!index.has(wire.from) || !index.has(wire.to)) return
    // The engine sees a wire that closes a loop the other way round, so
    // that everything it is given flows one way.
    const [source, target] = wire.against ? [wire.to, wire.from] : [wire.from, wire.to]
    graph.edges!.push({
      id: `${name}/w${position}`,
      sources: [`${node(source)}:out`],
      targets: [`${node(target)}:in`],
    })
  })
  for (const id of input.entries) {
    if (!index.has(id)) continue
    graph.edges!.push({
      id: `${name}/in${index.get(id)}`,
      sources: [`${name}:in`],
      targets: [`${node(id)}:in`],
    })
  }
  for (const id of input.exits) {
    if (!index.has(id)) continue
    graph.edges!.push({
      id: `${name}/out${index.get(id)}`,
      sources: [`${node(id)}:out`],
      targets: [`${name}:out`],
    })
  }
  return { input, rows, flow, rest, pills, graph }
}

/** The points of an edge the engine routed, start to end. */
function pointsOf(graph: ElkNode, id: string): Point[] | null {
  const edge = graph.edges?.find((candidate) => candidate.id === id)
  const sections = edge?.sections
  if (!sections || sections.length === 0) return null
  const points: Point[] = []
  for (const section of sections) {
    points.push(section.startPoint, ...(section.bendPoints ?? []), section.endPoint)
  }
  return points.map((point) => ({ x: point.x, y: point.y }))
}

/** Drops repeated points and points in the middle of a straight run. */
export function tidy(points: Point[]): Point[] {
  const out: Point[] = []
  for (const raw of points) {
    const point = { x: Math.round(raw.x * 2) / 2, y: Math.round(raw.y * 2) / 2 }
    const last = out[out.length - 1]
    if (last && last.x === point.x && last.y === point.y) continue
    const before = out[out.length - 2]
    if (
      before &&
      last &&
      ((before.x === last.x && last.x === point.x) || (before.y === last.y && last.y === point.y))
    ) {
      out[out.length - 1] = point
      continue
    }
    out.push(point)
  }
  return out
}

/** Puts the blocks together, once the engine has laid out the flow. */
export function finish(plan: Plan, flowed: ElkNode | null): ContainerResult {
  const { input, rows, flow, rest, pills } = plan
  const gap = SIZE.gap
  const positions: Record<string, Point> = {}
  const left = gap
  let y = SIZE.title + gap
  let contentWidth = Math.max(0, input.titleWidth - 2 * gap)

  // 1. Rows, in one column, all as wide as the widest.
  const columnWidth = rows.reduce((widest, row) => Math.max(widest, row.width), 0)
  for (const row of rows) {
    positions[row.id] = { x: left, y }
    y += row.height
  }
  if (rows.length > 0) {
    contentWidth = Math.max(contentWidth, columnWidth)
    y += gap
  }

  // 2. The flow, where the engine put it.
  const flowTop = y
  const flowWidth = flowed?.width ?? 0
  if (flowed && flow.length > 0) {
    flow.forEach((child, position) => {
      const placed = flowed.children?.find((node) => node.id === `${flowed.id}/${position}`)
      positions[child.id] = { x: left + (placed?.x ?? 0), y: flowTop + (placed?.y ?? 0) }
    })
    contentWidth = Math.max(contentWidth, flowWidth)
    y += (flowed.height ?? 0) + gap
  }

  // 3. The rest, in rows that wrap. Wide enough to sit under the blocks
  // above, and otherwise about as wide as a screen is.
  const shelfTop = new Map<string, number>()
  if (rest.length > 0) {
    const area = rest.reduce((sum, child) => sum + (child.width + gap) * (child.height + gap), 0)
    const widest = rest.reduce((most, child) => Math.max(most, child.width), 0)
    const target = Math.max(contentWidth, widest, Math.sqrt(area * 1.6))
    let x = 0
    let shelfHeight = 0
    let used = 0
    for (const child of rest) {
      if (x > 0 && x + child.width > target) {
        y += shelfHeight + gap
        x = 0
        shelfHeight = 0
      }
      positions[child.id] = { x: left + x, y }
      shelfTop.set(child.id, y)
      x += child.width + gap
      used = Math.max(used, x - gap)
      shelfHeight = Math.max(shelfHeight, child.height)
    }
    y += shelfHeight + gap
    contentWidth = Math.max(contentWidth, used)
  }

  // 4. Small marks, packed tight, as wide as what is above them.
  if (pills.length > 0) {
    const step = SIZE.pillWidth + SIZE.pillGap
    const perRow = Math.max(
      Math.floor((contentWidth + SIZE.pillGap) / step),
      Math.ceil(Math.sqrt(pills.length * 2)),
    )
    pills.forEach((pill, position) => {
      positions[pill.id] = {
        x: left + (position % perRow) * step,
        y: y + Math.floor(position / perRow) * (SIZE.pillHeight + SIZE.pillGap),
      }
    })
    const pillRows = Math.ceil(pills.length / perRow)
    y += pillRows * (SIZE.pillHeight + SIZE.pillGap) - SIZE.pillGap + gap
    contentWidth = Math.max(contentWidth, Math.min(pills.length, perRow) * step - SIZE.pillGap)
  }

  const width = Math.ceil(contentWidth + 2 * gap)
  const height = Math.ceil(input.children.length > 0 ? y : SIZE.title)

  // Lanes just inside the left and right borders, and the two doors.
  const door = SIZE.title / 2
  const laneIn = gap / 2
  const laneOut = width - gap / 2
  const wayIn = (through: Point[]): Point[] =>
    tidy([{ x: 0, y: door }, { x: laneIn, y: door }, ...through])
  const wayOut = (through: Point[]): Point[] =>
    tidy([...through, { x: laneOut, y: door }, { x: width, y: door }])

  const byId = new Map(input.children.map((child) => [child.id, child]))
  const flowIndex = new Map(flow.map((child, position) => [child.id, position]))
  const moved = (points: Point[]): Point[] =>
    points.map((point) => ({ x: left + point.x, y: flowTop + point.y }))

  const wires = input.wires.map((wire, position) => {
    if (!flowed || !flowIndex.has(wire.from) || !flowIndex.has(wire.to)) return null
    const points = pointsOf(flowed, `${flowed.id}/w${position}`)
    if (!points) return null
    const route = tidy(moved(points))
    return wire.against ? route.reverse() : route
  })

  const entries: Record<string, Point[]> = {}
  for (const id of input.entries) {
    const child = byId.get(id)
    const at = positions[id]
    if (!child || !at) continue
    const portY = at.y + portOffset(child.row)
    if (child.row) {
      entries[id] = wayIn([{ x: laneIn, y: portY }, { x: at.x, y: portY }])
    } else if (flowIndex.has(id)) {
      const points = flowed && pointsOf(flowed, `${flowed.id}/in${flowIndex.get(id)}`)
      if (!points) continue
      const inside = moved(points)
      entries[id] = wayIn([{ x: laneIn, y: inside[0].y }, ...inside])
    } else {
      const above = (shelfTop.get(id) ?? at.y) - gap / 2
      const beside = at.x - gap / 2
      entries[id] = wayIn([
        { x: laneIn, y: above },
        { x: beside, y: above },
        { x: beside, y: portY },
        { x: at.x, y: portY },
      ])
    }
  }

  const exits: Record<string, Point[]> = {}
  for (const id of input.exits) {
    const child = byId.get(id)
    const at = positions[id]
    if (!child || !at) continue
    const portY = at.y + portOffset(child.row)
    if (child.row) {
      exits[id] = wayOut([{ x: at.x + columnWidth, y: portY }, { x: laneOut, y: portY }])
    } else if (flowIndex.has(id)) {
      const points = flowed && pointsOf(flowed, `${flowed.id}/out${flowIndex.get(id)}`)
      if (!points) continue
      const inside = moved(points)
      exits[id] = wayOut([...inside, { x: laneOut, y: inside[inside.length - 1].y }])
    } else {
      const above = (shelfTop.get(id) ?? at.y) - gap / 2
      const beside = at.x + child.width + gap / 2
      exits[id] = wayOut([
        { x: at.x + child.width, y: portY },
        { x: beside, y: portY },
        { x: beside, y: above },
        { x: laneOut, y: above },
      ])
    }
  }

  return { width, height, positions, rowWidth: columnWidth, wires, entries, exits }
}
