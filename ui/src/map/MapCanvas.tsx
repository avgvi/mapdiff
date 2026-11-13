// The map itself: boxes and wires in map coordinates, inside one element
// that is moved and scaled as the reader pans and zooms.
//
// Moving the view only changes that element's transform. What is drawn is
// worked out again only when the view leaves the area it was worked out
// for, or the zoom has changed enough that more or less detail is due.

import {
  forwardRef,
  memo,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { SIZE, codeLines, wordOf } from '../layout/sizes.ts'
import type { ChangeDetail } from '../api'
import type { Box, MapNode, Point, WireRoute } from '../layout/types.ts'
import { covers, fitRect, panBy, viewOf, zoomAt, type Camera, type Rect, type Size } from './camera.ts'
import { useCode, type Source } from './code.ts'
import { chipOf, reviewedIn, type Diff } from './overlay.ts'
import { connect } from './paths.ts'
import { boxAt, fileOf, type Scene } from './scene.ts'
import { visibleIn, type LabelMode, type Visible } from './visible.ts'

/** A path drawn over the map for the selected box. */
export interface Link {
  key: string
  points: Point[]
  /** `in`: something uses the selected box. `out`: it uses something. */
  direction: 'in' | 'out'
}

export interface Highlight {
  selected: string
  /** Boxes joined to the selected one. */
  near: Set<string>
  /** Boxes around the selected one or around one joined to it. */
  around: Set<string>
  links: Link[]
}

/** The change being read: its box, and its lines before and after. */
export interface Pane {
  /** The box it belongs to. */
  id: string
  detail: ChangeDetail
}

/** A comparison drawn on the map. */
export interface DiffLayer {
  diff: Diff
  /** Review marks, by the key of a change. */
  reviewed: Set<string>
  /** Where the text after the change comes from, and the text before it. */
  after: Source
  before: Source
  pane: Pane | null
}

export interface CanvasHandle {
  camera(): Camera
  screen(): Size
  /** Moves the view. With `animate`, over a short time. */
  look(camera: Camera, animate?: boolean): void
  fit(): void
  zoom(factor: number): void
  pan(dx: number, dy: number): void
  /** Brings a box into view, zooming out only as far as it needs. */
  show(id: string): void
  /** The deepest box under the middle of the screen. */
  centre(): string | null
}

interface Props {
  scene: Scene
  highlight: Highlight | null
  /** Set when a comparison is drawn. */
  layer: DiffLayer | null
  /** Where code text comes from when no comparison is drawn. */
  source: Source
  /** Code boxes that show all of their text. */
  opened: ReadonlySet<string>
  /** The reader asked for the rest of a code box's text, or for less of it. */
  onMore(id: string): void
  onSelect(id: string | null): void
  onHover(id: string | null, at: { x: number; y: number }): void
  /** The reader zoomed in on something with more inside, or far out. */
  onWish(direction: 'deeper' | 'shallower', under: string | null): void
  onDrawn(count: number): void
  /** The reader asked for the trace view of a box. */
  onOpen(id: string): void
}

interface DrawnFor {
  visible: Visible
  view: Rect
  scale: number
}

const NOTHING: Visible['boxes'] = []

function pathOf(points: Point[], dx = 0, dy = 0): string {
  return points
    .map((point, position) => `${position === 0 ? 'M' : 'L'}${point.x + dx} ${point.y + dy}`)
    .join('')
}

/** The middle of the longest straight piece: where a count is written. */
function middleOf(points: Point[]): Point {
  let best = { x: points[0].x, y: points[0].y }
  let longest = -1
  for (let i = 1; i < points.length; i++) {
    const length = Math.abs(points[i].x - points[i - 1].x) + Math.abs(points[i].y - points[i - 1].y)
    if (length > longest) {
      longest = length
      best = { x: (points[i].x + points[i - 1].x) / 2, y: (points[i].y + points[i - 1].y) / 2 }
    }
  }
  return best
}

interface CodeProps {
  file: string
  node: MapNode
  source: Source
  /** Lines that the change added, by their number in the file. */
  inserted: Set<number> | null
  /** All of its lines are shown, not the first of them. */
  whole: boolean
  /** A line of the file to have in view when the text is cut: where the change being read starts. */
  focus: number | null
}

const CodeText = memo(function CodeText({ file, node, source, inserted, whole, focus }: CodeProps) {
  const lines = useCode(file, source)
  if (!lines) return null
  const { shown, hidden, foot } = codeLines(node, whole)
  // A cut block starts at its first line, unless a change further down is
  // being read: then it starts a little above that, and says how much is
  // above. That note takes the place of one line of text.
  let skipped = 0
  if (hidden > 0 && focus !== null) {
    skipped = Math.max(0, Math.min(focus - node.line - 4, node.lines - shown))
  }
  const first = node.line - 1 + skipped + (skipped > 0 ? 1 : 0)
  const count = shown - (skipped > 0 ? 1 : 0)
  const below = node.lines - skipped - shown
  return (
    <pre className="mcode-text">
      {skipped > 0 && <div className="mcode-more">… {skipped + 1} lines above</div>}
      {lines.slice(first, first + count).map((line, row) => (
        <div key={row} className={inserted?.has(first + 1 + row) ? 'is-inserted' : undefined}>
          {line.length === 0
            ? ' '
            : line.map((span, position) => (
                <span key={position} className={span.class ? `tok-${span.class}` : undefined}>
                  {span.text}
                </span>
              ))}
        </div>
      ))}
      {foot && (
        // Opens the rest of the text in place, or closes it again.
        <button type="button" className="mcode-more" data-more={node.id}>
          {whole
            ? hidden > 0
              ? `Show fewer lines (${hidden} more are not shown)`
              : 'Show fewer lines'
            : `… ${skipped > 0 ? below : hidden} more lines${skipped > 0 ? ' below' : ''} (show all)`}
        </button>
      )}
    </pre>
  )
})

interface BoxProps {
  box: Box
  node: MapNode
  open: boolean
  label: LabelMode
  text: boolean
  state: 'selected' | 'near' | 'around' | 'plain'
  file: string | null
  source: Source
  /** How it changed, when a comparison is drawn. */
  tone: string | null
  /** It did not change itself but holds something that did. */
  holds: boolean
  /** What its chip says: counts of changes, or lines added and removed. */
  chip: string | null
  /** Everything to review in it has been reviewed. */
  done: boolean
  /** It did not change and uses something whose change it can see. */
  impacted: boolean
  /** It did not change and touches something that did. */
  touches: boolean
  inserted: Set<number> | null
  whole: boolean
  focus: number | null
}

const BoxView = memo(function BoxView({
  box,
  node,
  open,
  label,
  text,
  state,
  file,
  source,
  tone,
  holds,
  chip,
  done,
  impacted,
  touches,
  inserted,
  whole,
  focus,
}: BoxProps) {
  const word = wordOf(node)
  const classes = [
    'mbox',
    `d-${box.detail}`,
    `k-${node.kind}`,
    box.detail === 'container' ? (open ? 'is-open' : 'is-closed') : '',
    state === 'plain' ? '' : `is-${state}`,
    box.presence === 'both' ? '' : `p-${box.presence}`,
    tone ? `t-${tone}` : '',
    holds ? 'holds-change' : '',
    done ? 'is-reviewed' : '',
    impacted ? 'is-impacted' : '',
    touches ? 'touches-change' : '',
    text ? 'has-text' : '',
  ]
  return (
    <div
      className={classes.filter(Boolean).join(' ')}
      data-id={box.id}
      title={box.detail === 'pill' ? node.label : undefined}
      style={{ left: box.x, top: box.y, width: box.width, height: box.height }}
    >
      {text ? (
        // Over code, the title is a bar of its own: clicking the code
        // selects text, so the bar is what stands for the box, and it says
        // what can be done with it.
        <div className="mbar" title="Click to see who uses it and what it uses">
          <span className="mbar-name">
            {word && <span className="mword">{word} </span>}
            {node.label}
          </span>
          <button type="button" className="mbar-tool" title="Show who uses it and what it uses, on the map and in the panel">
            Links
          </button>
          <button type="button" className="mbar-tool" data-open={box.id} title="Its code with its callers and what it calls, side by side (T)">
            Trace
          </button>
        </div>
      ) : (
        label !== 'none' && (
          <span className={`mlabel mlabel-${label}`}>
            {word && <span className="mword">{word} </span>}
            {node.label}
          </span>
        )
      )}
      {chip && label !== 'none' && <span className="mchip">{chip}</span>}
      {text && file && <CodeText file={file} node={node} source={source} inserted={inserted} whole={whole} focus={focus} />}
    </div>
  )
})

interface WiresProps {
  id: string
  origin: Box
  wires: WireRoute[]
  view: Rect
  counts: boolean
  selected: string | null
}

const WireLayer = memo(function WireLayer({ id, origin, wires, view, counts, selected }: WiresProps) {
  const drawn = wires.filter((wire) => {
    if (!wire.route) return false
    // Leave out wires that are wholly off screen.
    let [left, top, right, bottom] = [Infinity, Infinity, -Infinity, -Infinity]
    for (const point of wire.route) {
      left = Math.min(left, point.x)
      right = Math.max(right, point.x)
      top = Math.min(top, point.y)
      bottom = Math.max(bottom, point.y)
    }
    return (
      left + origin.x < view.x + view.width &&
      right + origin.x > view.x &&
      top + origin.y < view.y + view.height &&
      bottom + origin.y > view.y
    )
  })
  if (drawn.length === 0) return null
  return (
    <svg className="mwires" data-container={id} style={{ left: origin.x, top: origin.y }}>
      {drawn.map((wire) => {
        const touches = selected !== null && (wire.from === selected || wire.to === selected)
        const middle = wire.count > 1 && (counts || touches) ? middleOf(wire.route!) : null
        return (
          <g
            key={`${wire.from}>${wire.to}`}
            className={`mwire w-${wire.kind}${touches ? ' is-lit' : ''}${
              wire.presence === 'both' ? '' : ` p-${wire.presence}`
            }`}
          >
            <path d={pathOf(wire.route!)} markerEnd="url(#mwire-arrow)" />
            {middle && (
              <text x={middle.x} y={middle.y}>
                {wire.count}
              </text>
            )}
          </g>
        )
      })}
    </svg>
  )
})

export const MapCanvas = forwardRef<CanvasHandle, Props>(function MapCanvas(
  { scene, highlight, layer, source, opened, onMore, onSelect, onHover, onWish, onDrawn, onOpen },
  handle,
) {
  const frame = useRef<HTMLDivElement>(null)
  const world = useRef<HTMLDivElement>(null)
  const camera = useRef<Camera>({ x: 0, y: 0, scale: 1 })
  const screen = useRef<Size>({ width: 1, height: 1 })
  const drawnFor = useRef<DrawnFor | null>(null)
  const animation = useRef(0)
  const settle = useRef(0)
  const [drawn, setDrawn] = useState<DrawnFor | null>(null)
  const sceneRef = useRef(scene)

  /** Works out what to draw for the current view, with room to move in. */
  const redraw = useCallback(() => {
    const view = viewOf(camera.current, screen.current)
    const roomy: Rect = {
      x: view.x - view.width * 0.6,
      y: view.y - view.height * 0.6,
      width: view.width * 2.2,
      height: view.height * 2.2,
    }
    const next = {
      visible: visibleIn(sceneRef.current, roomy, camera.current.scale),
      view: roomy,
      scale: camera.current.scale,
    }
    drawnFor.current = next
    setDrawn(next)
  }, [])

  /** Moves the map element; redraws only when the view has left what was drawn. */
  const apply = useCallback(
    (next: Camera) => {
      camera.current = next
      const element = world.current
      if (element) {
        element.style.transform = `translate(${-next.x * next.scale}px, ${-next.y * next.scale}px) scale(${next.scale})`
        element.style.setProperty('--inv', String(1 / next.scale))
      }
      if (!drawnFor.current || !covers(drawnFor.current, next, screen.current)) redraw()
    },
    [redraw],
  )

  const look = useCallback(
    (target: Camera, animate = false) => {
      cancelAnimationFrame(animation.current)
      if (!animate) return apply(target)
      const from = camera.current
      const started = performance.now()
      const step = (now: number) => {
        const t = Math.min(1, (now - started) / 260)
        const eased = 1 - (1 - t) ** 3
        // Zoom is eased in its logarithm, so it looks even.
        const scale = from.scale * (target.scale / from.scale) ** eased
        apply({
          scale,
          x: from.x + (target.x - from.x) * eased,
          y: from.y + (target.y - from.y) * eased,
        })
        if (t < 1) animation.current = requestAnimationFrame(step)
        // What is drawn was last worked out somewhere along the way, for a
        // zoom up to a quarter off. Now that the view has stopped, work it
        // out for where it is: names appear that were too small before.
        else redraw()
      }
      animation.current = requestAnimationFrame(step)
    },
    [apply, redraw],
  )

  const whole = useCallback(
    (): Rect => ({ x: 0, y: 0, width: sceneRef.current.layout.width, height: sceneRef.current.layout.height }),
    [],
  )

  useImperativeHandle(
    handle,
    () => ({
      camera: () => camera.current,
      screen: () => screen.current,
      look,
      fit: () => look(fitRect(whole(), screen.current), true),
      zoom: (factor) =>
        look(zoomAt(camera.current, factor, { x: screen.current.width / 2, y: screen.current.height / 2 }), true),
      pan: (dx, dy) => apply(panBy(camera.current, dx, dy)),
      show: (id) => {
        const box = sceneRef.current.boxes[id]
        if (!box) return
        const size = screen.current
        if (box.detail === 'code') {
          // Code is brought to a size it can be read at, whatever its
          // length: as wide as fits, its top near the top of the view.
          const scale = Math.min(1, Math.max(0.55, (size.width - 80) / box.width))
          look(
            {
              scale,
              x: box.x - Math.max(40, (size.width - box.width * scale) / 2) / scale,
              y: box.y - 36 / scale,
            },
            true,
          )
          return
        }
        // Anything else is shown whole, no closer than it is now unless it
        // is too small to make out.
        const most = Math.max(Math.min(camera.current.scale, 1), 0.6)
        look(fitRect(box, size, 80, most), true)
      },
      centre: () => {
        const view = viewOf(camera.current, screen.current)
        return boxAt(sceneRef.current, view.x + view.width / 2, view.y + view.height / 2)
      },
    }),
    [apply, look, whole],
  )

  // A new map: everything drawn so far is for the old one. The frame is
  // measured here too, so that whoever placed the map can fit the view to
  // it straight away.
  useLayoutEffect(() => {
    sceneRef.current = scene
    drawnFor.current = null
    const element = frame.current
    if (element) screen.current = { width: element.clientWidth, height: element.clientHeight }
    redraw()
  }, [scene, redraw])

  useEffect(() => {
    const element = frame.current
    if (!element) return
    const measure = () => {
      screen.current = { width: element.clientWidth, height: element.clientHeight }
      redraw()
    }
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [redraw])

  useEffect(() => {
    onDrawn(drawn?.visible.boxes.length ?? 0)
  }, [drawn, onDrawn])

  // Wheel: two fingers or a wheel pan, with Ctrl or Cmd (and a pinch) they zoom.
  useEffect(() => {
    const element = frame.current
    if (!element) return
    const wheel = (event: WheelEvent) => {
      event.preventDefault()
      cancelAnimationFrame(animation.current)
      const bounds = element.getBoundingClientRect()
      const at = { x: event.clientX - bounds.left, y: event.clientY - bounds.top }
      if (event.ctrlKey || event.metaKey) {
        const factor = Math.exp(-event.deltaY * 0.01)
        apply(zoomAt(camera.current, factor, at))
        // The same once a zoom by hand has come to rest.
        window.clearTimeout(settle.current)
        settle.current = window.setTimeout(redraw, 140)
        const current = camera.current
        const under = boxAt(sceneRef.current, current.x + at.x / current.scale, current.y + at.y / current.scale)
        if (factor > 1 && current.scale > 1.7) onWish('deeper', under)
        const fits = fitRect(whole(), screen.current).scale
        if (factor < 1 && current.scale < fits * 0.5) onWish('shallower', under)
      } else {
        apply(panBy(camera.current, event.deltaX, event.deltaY))
      }
    }
    element.addEventListener('wheel', wheel, { passive: false })
    return () => element.removeEventListener('wheel', wheel)
  }, [apply, redraw, onWish, whole])

  // Dragging pans. A press and release in one place is a click.
  const drag = useRef<{ x: number; y: number; moved: boolean; id: number } | null>(null)
  const idUnder = (target: EventTarget | null): string | null =>
    target instanceof Element ? (target.closest('[data-id]')?.getAttribute('data-id') ?? null) : null

  /**
   * The box the pointer is asking about. A box that shows what is in it
   * (an open container, code that can be read) only answers on its title
   * bar: over its content the reader is looking at the content, and a card
   * would be in the way.
   */
  const hoverUnder = (event: React.PointerEvent): string | null => {
    const element = event.target instanceof Element ? event.target.closest('[data-id]') : null
    if (!element) return null
    const showsContent = element.classList.contains('is-open') || element.querySelector(':scope > .mcode-text') !== null
    if (showsContent) {
      const below = (event.clientY - element.getBoundingClientRect().top) / camera.current.scale
      if (below > SIZE.title) return null
    }
    return element.getAttribute('data-id')
  }

  const pointerDown = (event: React.PointerEvent) => {
    if (event.button !== 0) return
    const target = event.target instanceof Element ? event.target : null
    // Over code a press is about the text: it selects text, and neither
    // moves the map nor selects the box. A box is chosen by its title bar.
    // The buttons on a box are buttons, not part of the map.
    if (target?.closest('.mcode-text, .mtools, [data-open]')) return
    cancelAnimationFrame(animation.current)
    drag.current = { x: event.clientX, y: event.clientY, moved: false, id: event.pointerId }
  }
  const pointerMove = (event: React.PointerEvent) => {
    const held = drag.current
    if (!held) {
      onHover(hoverUnder(event), { x: event.clientX, y: event.clientY })
      return
    }
    const [dx, dy] = [event.clientX - held.x, event.clientY - held.y]
    if (!held.moved && Math.abs(dx) + Math.abs(dy) < 4) return
    if (!held.moved) {
      held.moved = true
      frame.current?.setPointerCapture(held.id)
      frame.current?.classList.add('is-dragging')
      onHover(null, { x: 0, y: 0 })
    }
    apply(panBy(camera.current, -dx, -dy))
    held.x = event.clientX
    held.y = event.clientY
  }
  const pointerUp = (event: React.PointerEvent) => {
    const held = drag.current
    drag.current = null
    frame.current?.classList.remove('is-dragging')
    if (!held || held.moved) return
    onSelect(idUnder(event.target))
  }

  /** A double click on a box opens its trace view; on code, it selects a word as usual. */
  const doubleClick = (event: React.MouseEvent) => {
    const target = event.target instanceof Element ? event.target : null
    if (!target || target.closest('.mcode-text') || target.closest('.mtools')) return
    const id = idUnder(target)
    if (id !== null && id !== scene.layout.root) onOpen(id)
  }

  const boxes = drawn?.visible.boxes ?? NOTHING
  const files = useMemo(() => {
    const out = new Map<string, string | null>()
    for (const box of boxes) {
      if (!box.text) continue
      // A ghost file whose name was taken again carries a mark in its ID.
      const file = fileOf(scene, box.id)
      out.set(box.id, file?.endsWith(' (before)') ? file.slice(0, -' (before)'.length) : file)
    }
    return out
  }, [boxes, scene])

  // The lines the selected change added, to tint in its code box.
  const inserted = useMemo(() => {
    if (!layer?.pane) return null
    const lines = new Set<number>()
    for (const row of layer.pane.detail.rows) {
      if (row.tag === 'insert' && row.new_line !== null) lines.add(row.new_line)
    }
    return lines
  }, [layer?.pane])

  // Where the change being read starts, in the file after the change: its
  // first added line, or for lines only taken out, the line after them.
  const focus = useMemo(() => {
    if (!layer?.pane) return null
    const rows = layer.pane.detail.rows
    const at = rows.findIndex((row) => row.tag !== 'equal')
    if (at === -1) return null
    for (let next = at; next < rows.length; next++) {
      const line = rows[next].new_line
      if (line !== null) return line
    }
    return null
  }, [layer?.pane])

  const moves = useMemo(
    () =>
      (layer?.diff.moves ?? []).flatMap((move) => {
        const points = connect(scene, move.from, move.to)
        return points ? [{ key: `${move.from}>${move.to}`, points }] : []
      }),
    [layer?.diff, scene],
  )

  /** What a comparison adds to a box: its colour, its chip, its marks. */
  const changeOf = (id: string) => {
    const none = { tone: null, holds: false, chip: null, done: false, impacted: false, touches: false }
    if (!layer) return none
    const { diff, reviewed } = layer
    const mark = diff.marks[id]
    const impacted = id in diff.impacted
    const touches = !impacted && diff.near.has(id)
    if (!mark) return { ...none, impacted, touches }
    const progress = reviewedIn(diff, id, reviewed)
    const own = mark.own === null ? null : diff.changes[mark.own]
    // One change of its own: lines. Otherwise: how many changes of each kind.
    let chip: string | null = null
    if (own && mark.inside.length <= 1) {
      const { added, removed } = own.total
      chip = [added > 0 ? `+${added}` : '', removed > 0 ? `−${removed}` : ''].filter(Boolean).join(' ')
    } else if (mark.inside.length > 0) {
      chip = chipOf(mark)
      if (progress.done > 0 && progress.done < progress.of) chip += `  ${progress.done}/${progress.of} ✓`
    }
    const done = progress.of > 0 && progress.done === progress.of
    if (done) chip = chip ? `${chip} ✓` : '✓'
    return {
      tone: mark.tone,
      holds: mark.tone === null && mark.inside.length > 0,
      chip: chip === '' ? null : chip,
      done,
      impacted: false,
      touches: false,
    }
  }
  const pane = layer?.pane && scene.boxes[layer.pane.id] ? layer.pane : null
  const stateOf = (id: string): BoxProps['state'] => {
    if (!highlight) return 'plain'
    if (id === highlight.selected) return 'selected'
    if (highlight.near.has(id)) return 'near'
    if (highlight.around.has(id)) return 'around'
    // What is inside a lit box is lit with it.
    for (let outer = scene.boxes[id]?.parent ?? null; outer !== null; outer = scene.boxes[outer]?.parent ?? null) {
      if (outer === highlight.selected || highlight.near.has(outer)) return 'around'
    }
    return 'plain'
  }

  return (
    <div
      className="mframe"
      ref={frame}
      onPointerDown={pointerDown}
      onPointerMove={pointerMove}
      onPointerUp={pointerUp}
      onPointerLeave={() => onHover(null, { x: 0, y: 0 })}
      onDoubleClick={doubleClick}
      onClick={(event) => {
        const target = event.target instanceof Element ? event.target : null
        const more = target?.closest('[data-more]')
        if (more) onMore(more.getAttribute('data-more')!)
        const tool = target?.closest('[data-tool]')
        if (tool && highlight) onOpen(highlight.selected)
        const open = target?.closest('[data-open]')
        if (open) onOpen(open.getAttribute('data-open')!)
      }}
    >
      <svg className="mdefs" aria-hidden="true">
        <defs>
          <marker id="mwire-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="5" markerHeight="5" orient="auto-start-reverse">
            <path d="M0 0L10 5L0 10z" fill="context-stroke" />
          </marker>
        </defs>
      </svg>
      <div className={`mworld${highlight ? ' is-selecting' : ''}`} ref={world}>
        {boxes.map((box) =>
          scene.boxes[box.id] ? (
            <BoxView
              key={box.id}
              box={scene.boxes[box.id]}
              node={scene.nodes[box.id]}
              open={box.open}
              label={box.label}
              text={box.text}
              state={stateOf(box.id)}
              file={files.get(box.id) ?? null}
              source={
                layer ? (scene.boxes[box.id].presence === 'removed' ? layer.before : layer.after) : source
              }
              inserted={pane?.id === box.id ? inserted : null}
              whole={opened.has(box.id)}
              focus={pane?.id === box.id ? focus : null}
              {...changeOf(box.id)}
            />
          ) : null,
        )}
        {drawn?.visible.wires.map(({ id, counts }) =>
          scene.layout.containers[id] ? (
            <WireLayer
              key={id}
              id={id}
              origin={scene.boxes[id]}
              wires={scene.layout.containers[id].wires}
              view={drawn.view}
              counts={counts}
              selected={highlight?.selected ?? null}
            />
          ) : null,
        )}
        {highlight &&
          scene.boxes[highlight.selected] &&
          highlight.selected !== scene.layout.root &&
          !boxes.some((box) => box.id === highlight.selected && box.text) && (
          // What can be done with the selected box, on a tab above its
          // top right corner, at a size that does not change with the zoom.
          <div
            className="mtools"
            style={{
              left: scene.boxes[highlight.selected].x + scene.boxes[highlight.selected].width,
              top: scene.boxes[highlight.selected].y,
            }}
          >
            <button type="button" data-tool="trace" title="Its code, with who uses it and what it uses (T, or double-click its title)">
              Trace
            </button>
          </div>
        )}
        {moves.length > 0 && (
          <svg className="mwires mmoves" style={{ left: 0, top: 0 }}>
            {moves.map((move) => (
              <path key={move.key} className="mmove" d={pathOf(move.points)} markerEnd="url(#mwire-arrow)" />
            ))}
          </svg>
        )}
        {highlight && highlight.links.length > 0 && (
          <svg className="mwires mlinks" style={{ left: 0, top: 0 }}>
            {highlight.links.map((link) => (
              <path key={link.key} className={`mlink mlink-${link.direction}`} d={pathOf(link.points)} markerEnd="url(#mwire-arrow)" />
            ))}
          </svg>
        )}
      </div>
    </div>
  )
})
