// The path of a wire between two boxes anywhere on the map, put together
// from the pieces the layout made: the way out of each container on one
// side, the wire between two siblings at the top, the way in on the other.

import { portOffset } from '../layout/sizes.ts'
import type { Point } from '../layout/types.ts'
import { ancestorsOf, type Scene } from './scene.ts'

function shifted(points: Point[], by: Point): Point[] {
  return points.map((point) => ({ x: point.x + by.x, y: point.y + by.y }))
}

/** Where wires leave a box and where they arrive, on the map. */
export function doors(scene: Scene, id: string): { in: Point; out: Point } {
  const box = scene.boxes[id]
  const y = box.y + portOffset(box.detail === 'row')
  return { in: { x: box.x, y }, out: { x: box.x + box.width, y } }
}

/**
 * The path from one box to another, in map coordinates. Null when one is
 * inside the other, or either is not on the map.
 */
export function connect(scene: Scene, from: string, to: string): Point[] | null {
  if (!scene.boxes[from] || !scene.boxes[to] || from === to) return null
  const up = [...ancestorsOf(scene, from), from]
  const down = [...ancestorsOf(scene, to), to]
  let shared = 0
  while (shared < up.length && shared < down.length && up[shared] === down[shared]) shared++
  // One contains the other: there is nothing to draw between them.
  if (shared === up.length || shared === down.length || shared === 0) return null

  const points: Point[] = []
  const add = (piece: Point[]) => points.push(...piece)

  // Out of every container on the way up.
  for (let depth = up.length - 1; depth > shared; depth--) {
    const [child, container] = [up[depth], up[depth - 1]]
    const route = scene.layout.containers[container]?.exits[child]
    add(route ? shifted(route, scene.boxes[container]) : [doors(scene, child).out, doors(scene, container).out])
  }
  // Across, between the two children of the container that holds both.
  const top = up[shared - 1]
  const [left, right] = [up[shared], down[shared]]
  const wire = scene.layout.containers[top]?.wires.find(
    (candidate) => candidate.from === left && candidate.to === right,
  )
  add(wire?.route ? shifted(wire.route, scene.boxes[top]) : [doors(scene, left).out, doors(scene, right).in])
  // Into every container on the way down.
  for (let depth = shared + 1; depth < down.length; depth++) {
    const [container, child] = [down[depth - 1], down[depth]]
    const route = scene.layout.containers[container]?.entries[child]
    add(route ? shifted(route, scene.boxes[container]) : [doors(scene, container).in, doors(scene, child).in])
  }

  // Pieces meet at the doors: drop the repeated points.
  return points.filter(
    (point, position) =>
      position === 0 || point.x !== points[position - 1].x || point.y !== points[position - 1].y,
  )
}
