// What is worth drawing: the boxes that are on screen and large enough to
// see, with as much detail as their size on screen allows.

import { SIZE } from '../layout/sizes.ts'
import type { Rect } from './camera.ts'
import type { Scene } from './scene.ts'

export type LabelMode =
  /** In the title bar, growing and shrinking with the map. */
  | 'title'
  /** At a fixed size on screen, for a box whose title bar is too small to read. */
  | 'float'
  | 'none'

export interface Drawn {
  id: string
  /** A container whose children are drawn. Closed, it is one plain shape. */
  open: boolean
  label: LabelMode
  /** A code box whose text is large enough to draw. */
  text: boolean
}

export interface Visible {
  boxes: Drawn[]
  /** Containers whose wires are drawn, and whether each wire shows how many references it stands for. */
  wires: { id: string; counts: boolean }[]
}

/** Sizes on screen, in pixels, below which detail is left out. */
export const LIMIT = {
  /** A box smaller than this in both directions is not drawn at all. */
  box: 2.5,
  /** A container is opened when it is at least this wide and this high. */
  openWidth: 36,
  openHeight: 20,
  /** A title bar is read in place when it is at least this high: its 12 px text is then 9 px. */
  title: 21,
  /** A floating label needs this much room. */
  floatWidth: 44,
  floatHeight: 15,
  /**
   * Wires are drawn in a container whose typical child is at least this
   * wide, and a quarter of this high: the boxes they join can be told apart.
   */
  wires: 36,
  /** The same, for writing a count on a wire. */
  counts: 64,
  /** Source text is drawn when a line is at least this high. */
  codeLine: 6,
} as const

function intersects(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

/** Walks the map from the root and collects what to draw for a view. */
export function visibleIn(scene: Scene, view: Rect, scale: number): Visible {
  const boxes: Drawn[] = []
  const wires: Visible['wires'] = []
  const walk = (id: string) => {
    const box = scene.boxes[id]
    if (!intersects(box, view)) return
    const [width, height] = [box.width * scale, box.height * scale]
    if (Math.max(width, height) < LIMIT.box) return

    const kids = scene.children[id]
    const open =
      box.detail === 'container' && width >= LIMIT.openWidth && height >= LIMIT.openHeight
    const bar = (box.detail === 'row' ? SIZE.row : SIZE.title) * scale
    let label: LabelMode = 'none'
    if (box.detail === 'pill') label = 'none'
    else if (bar >= LIMIT.title) label = 'title'
    else if (box.detail !== 'row' && width >= LIMIT.floatWidth && height >= LIMIT.floatHeight) {
      label = 'float'
    }
    boxes.push({
      id,
      open,
      label,
      text: box.detail === 'code' && SIZE.codeLine * scale >= LIMIT.codeLine,
    })
    if (!open || !kids) return
    const typical = scene.childSize[id]
    if (typical) {
      const size = Math.min(typical.width, typical.height * 4) * scale
      if (size >= LIMIT.wires) wires.push({ id, counts: size >= LIMIT.counts })
    }
    for (const kid of kids) walk(kid)
  }
  walk(scene.layout.root)
  return { boxes, wires }
}
