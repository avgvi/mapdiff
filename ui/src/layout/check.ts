// Checks on a finished layout, for tests and for measuring real maps.

import type { Box, MapLayout, Point } from './types.ts'

/** Problems with where boxes are: overlapping siblings, children sticking out. */
export function boxProblems(layout: MapLayout): string[] {
  const problems: string[] = []
  const children = new Map<string, Box[]>()
  for (const box of Object.values(layout.boxes)) {
    if (box.parent === null) continue
    const list = children.get(box.parent) ?? []
    list.push(box)
    children.set(box.parent, list)
  }
  for (const [parent, list] of children) {
    const outer = layout.boxes[parent]
    for (const box of list) {
      if (box.x < 0 || box.y < 0 || box.x + box.width > outer.width || box.y + box.height > outer.height) {
        problems.push(`${box.id} sticks out of ${parent}`)
      }
    }
    // Sweep from the left; compare each box with those that start before it ends.
    const sorted = [...list].sort((a, b) => a.x - b.x)
    for (let i = 0; i < sorted.length; i++) {
      for (let j = i + 1; j < sorted.length && sorted[j].x < sorted[i].x + sorted[i].width; j++) {
        const [a, b] = [sorted[i], sorted[j]]
        if (a.y < b.y + b.height && b.y < a.y + a.height) {
          problems.push(`${a.id} overlaps ${b.id}`)
        }
      }
    }
  }
  return problems
}

/** True when a straight piece of a route runs through the inside of a box. */
function passesThrough(a: Point, b: Point, box: Box): boolean {
  const inset = 0.5
  const [left, right] = [box.x + inset, box.x + box.width - inset]
  const [top, bottom] = [box.y + inset, box.y + box.height - inset]
  const [minX, maxX] = [Math.min(a.x, b.x), Math.max(a.x, b.x)]
  const [minY, maxY] = [Math.min(a.y, b.y), Math.max(a.y, b.y)]
  return minX < right && maxX > left && minY < bottom && maxY > top
}

/**
 * Problems with how wires run: a piece that is not horizontal or vertical,
 * a route through a box it does not belong to, a route that does not start
 * and end where it should.
 */
export function routeProblems(layout: MapLayout): string[] {
  const problems: string[] = []
  const children = new Map<string, Box[]>()
  for (const box of Object.values(layout.boxes)) {
    if (box.parent === null) continue
    const list = children.get(box.parent) ?? []
    list.push(box)
    children.set(box.parent, list)
  }
  const near = (a: number, b: number) => Math.abs(a - b) <= 1

  for (const [id, container] of Object.entries(layout.containers)) {
    const inside = children.get(id) ?? []
    const outer = layout.boxes[id]
    const check = (name: string, route: Point[], ends: string[]) => {
      for (let i = 1; i < route.length; i++) {
        const [a, b] = [route[i - 1], route[i]]
        if (a.x !== b.x && a.y !== b.y) problems.push(`${name}: a piece is not straight`)
        for (const box of inside) {
          if (!ends.includes(box.id) && passesThrough(a, b, box)) {
            problems.push(`${name}: runs through ${box.id}`)
          }
        }
      }
    }
    for (const wire of container.wires) {
      if (!wire.route) continue
      const name = `${wire.from} -> ${wire.to}`
      check(name, wire.route, [wire.from, wire.to])
      const [from, to] = [layout.boxes[wire.from], layout.boxes[wire.to]]
      const [start, end] = [wire.route[0], wire.route[wire.route.length - 1]]
      // With the flow: out of the right side, into the left side. A wire
      // that closes a loop does the opposite.
      const [leaves, arrives] =
        wire.kind === 'cycle' ? [from.x, to.x + to.width] : [from.x + from.width, to.x]
      if (!near(start.x, leaves) || !near(end.x, arrives)) {
        problems.push(`${name}: does not join its two boxes`)
      }
    }
    for (const [child, route] of Object.entries(container.entries)) {
      check(`into ${child}`, route, [child])
      const box = layout.boxes[child]
      const [start, end] = [route[0], route[route.length - 1]]
      if (start.x !== 0 || !near(end.x, box.x)) problems.push(`into ${child}: wrong ends`)
    }
    for (const [child, route] of Object.entries(container.exits)) {
      check(`out of ${child}`, route, [child])
      const box = layout.boxes[child]
      const [start, end] = [route[0], route[route.length - 1]]
      if (end.x !== outer.width || !near(start.x, box.x + box.width)) {
        problems.push(`out of ${child}: wrong ends`)
      }
    }
  }
  return problems
}
