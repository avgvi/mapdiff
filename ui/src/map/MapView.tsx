// The map viewer: the repository as nested boxes, with wires for what uses
// what, at four levels of detail. With "Changes" on, a comparison is drawn
// on it: this is the visual reviewer.

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import {
  ApiError,
  fetchChanges,
  fetchDetail,
  fetchMap,
  fetchMeta,
  fetchNode,
  fetchReferences,
  fetchReviewed,
  saveReviewed,
  startIndexing,
  type ChangeDetail,
  type ChangeSet,
  type Hit,
  type Meta,
  type NodeInfo,
  type References,
  type Target,
  type Use,
} from '../api'
import { DiffView } from '../components/DiffView'
import { TargetPicker } from '../components/TargetPicker'
import { workerEngine } from '../layout/engine-worker.ts'
import { LayoutCache, layoutMap } from '../layout/layout.ts'
import type { Level, MapData } from '../layout/types.ts'
import { changeId, keysToMark, reviewKey } from '../model'
import { readTarget, writeTarget } from '../target'
import { fitRect, follow, toScreen, type Camera, type Rect } from './camera.ts'
import { ChangeList } from './ChangeList.tsx'
import { NOW, forgetCode, type Source } from './code.ts'
import { Inspector } from './Inspector.tsx'
import { MapCanvas, type CanvasHandle, type DiffLayer, type Highlight, type Link } from './MapCanvas.tsx'
import { buildDiff, isolate, type Diff } from './overlay.ts'
import { connect } from './paths.ts'
import { ancestorsOf, buildScene, type Scene } from './scene.ts'
import { SearchPalette } from './SearchPalette.tsx'
import { Trace } from './Trace.tsx'
import './map.css'

/** How references are found, as the bar says it and as its tooltip explains it. */
const REFERENCES: Record<References['by'], { name: string; means: string }> = {
  name: {
    name: 'by name',
    means: 'What uses what is found from names and imports. A method called on a value is not seen.',
  },
  type: {
    name: 'by type',
    means: "What uses what is found from names and from the language's own index, which also sees methods called on a value.",
  },
  mixed: {
    name: 'by type in part',
    means: 'Some projects in this repository have an index and some do not. Where there is none, a method called on a value is not seen.',
  },
}

const LEVELS: { level: Level; name: string; shows: string }[] = [
  { level: 'architecture', name: 'Architecture', shows: 'Directories, with files as small marks' },
  { level: 'package', name: 'Package', shows: 'Files' },
  { level: 'module', name: 'Module', shows: 'Types and functions' },
  { level: 'code', name: 'Code', shows: 'The text of every function' },
]

/** How much of the map is shown when a comparison is drawn. */
type Isolated = 'all' | 'changed' | 'near'

/** What to do with the view when another map arrives. */
type Arrival =
  | { kind: 'fit' }
  /** Keep a box where it is: the first of `chain` that the new map has. */
  | { kind: 'follow'; chain: string[]; rects: Record<string, Rect>; camera: Camera }
  | { kind: 'show'; id: string }
  | { kind: 'stay' }

interface Stop {
  level: Level
  id: string
}

interface Hover {
  id: string
  x: number
  y: number
}

/** A map as it came from the server, with the change set drawn on it. */
interface Loaded {
  data: MapData
  changes: ChangeSet | null
  target: Target | null
}

function HoverCard({ hover, info, onPick, onKeep, onLeave }: {
  hover: Hover
  info: NodeInfo | null
  onPick(action: 'callers' | 'dependencies' | 'open'): void
  onKeep(): void
  onLeave(): void
}) {
  // Beside the pointer, and never off the right or bottom edge.
  const left = Math.min(hover.x + 14, window.innerWidth - 380)
  const top = Math.min(hover.y + 16, window.innerHeight - 170)
  return (
    <div className="mhover" style={{ left, top }} onPointerEnter={onKeep} onPointerLeave={onLeave} role="tooltip">
      {info ? (
        <>
          <div className="mhover-title">
            <span className="mword">{info.lang_kind}</span> <strong>{info.label}</strong>
          </div>
          <div className="mhover-where">
            {info.path}
            {info.first_line > 0 && `:${info.first_line}`}
          </div>
          {info.signature && <pre className="mhover-signature">{info.signature}</pre>}
          <div className="mhover-actions">
            <button type="button" onClick={() => onPick('callers')}>
              Callers {info.callers.length}
            </button>
            <button type="button" onClick={() => onPick('dependencies')}>
              Dependencies {info.dependencies.length}
            </button>
            <button type="button" onClick={() => onPick('open')}>Open</button>
          </div>
        </>
      ) : (
        <div className="mhover-where">{hover.id}</div>
      )}
    </div>
  )
}

export function MapView() {
  // One engine and one layout cache for the life of the page: a level that
  // was shown before comes back at once.
  const engine = useMemo(() => workerEngine(), [])
  const cache = useMemo(() => new LayoutCache(), [])
  useEffect(() => () => engine.stop(), [engine])

  const canvas = useRef<CanvasHandle>(null)
  const [level, setLevel] = useState<Level>('package')
  const [auto, setAuto] = useState(true)
  // `#map/changes` draws the comparison the address names; `#map` the files as they are.
  const [changesOn, setChangesOn] = useState(() => window.location.hash.startsWith('#map/changes'))
  const [target, setTarget] = useState<Target>(readTarget)
  const [meta, setMeta] = useState<Meta | null>(null)
  const [isolated, setIsolated] = useState<Isolated>('all')
  const [loaded, setLoaded] = useState<Loaded | null>(null)
  const [view, setView] = useState<{ scene: Scene; diff: Diff | null; target: Target | null } | null>(null)
  const [busy, setBusy] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [reloads, setReloads] = useState(0)
  const [selected, setSelected] = useState<string | null>(null)
  const [info, setInfo] = useState<NodeInfo | null>(null)
  const [focus, setFocus] = useState<'callers' | 'dependencies' | null>(null)
  const [searching, setSearching] = useState(false)
  const [trace, setTrace] = useState<string | null>(null)
  const [drawnCount, setDrawnCount] = useState(0)
  const [history, setHistory] = useState<{ stops: Stop[]; at: number }>({ stops: [], at: -1 })
  const [hover, setHover] = useState<Hover | null>(null)
  const [hoverInfo, setHoverInfo] = useState<NodeInfo | null>(null)
  const [reviewed, setReviewed] = useState<Set<string>>(new Set())
  /** The change being read, as a position in the change set. */
  const [current, setCurrent] = useState<number | null>(null)
  const [callerFirst, setCallerFirst] = useState(false)
  const [detail, setDetail] = useState<{ of: number; detail: ChangeDetail } | null>(null)
  /** Code boxes opened past their cut. */
  const [whole, setWhole] = useState<ReadonlySet<string>>(new Set())
  /** The before and after of the change being read, under the map. */
  const [dock, setDock] = useState(true)
  const [references, setReferences] = useState<References | null>(null)
  const [indexAsked, setIndexAsked] = useState(0)

  const arrival = useRef<Arrival>({ kind: 'fit' })
  const infos = useRef(new Map<string, Promise<NodeInfo>>())
  const hoverTimer = useRef(0)
  const wished = useRef(0)
  const selectionChain = useRef<string[]>([])
  const wasIndexing = useRef(false)

  const scene = view?.scene ?? null
  const diff = view?.diff ?? null
  const shownTarget = changesOn ? target : null
  // One object per comparison, so that what depends on it is not redone on every render.
  const targetKey = JSON.stringify(shownTarget)
  const after = useMemo((): Source => ({ target: JSON.parse(targetKey) as Target | null, side: 'head' }), [targetKey])
  const before = useMemo((): Source => ({ target: after.target, side: 'base' }), [after])

  const nodeInfo = useCallback(
    (id: string, at: Level): Promise<NodeInfo> => {
      const key = `${targetKey}\n${at}\n${id}`
      let known = infos.current.get(key)
      if (!known) {
        known = fetchNode(id, at, after.target)
        known.catch(() => infos.current.delete(key))
        infos.current.set(key, known)
      }
      return known
    },
    [targetKey, after],
  )

  useEffect(() => {
    fetchMeta().then(setMeta).catch(() => {})
  }, [])
  useEffect(() => {
    fetchReviewed()
      .then((keys) => setReviewed(new Set(keys)))
      .catch(() => {})
  }, [reloads])

  const reload = useCallback(() => {
    infos.current.clear()
    forgetCode()
    arrival.current = { kind: 'stay' }
    setBusy(true)
    setReloads((count) => count + 1)
  }, [])

  // How references are found. While an index is being made this is asked
  // again, and when it is done the map is read again to gain what it found.
  useEffect(() => {
    let live = true
    let timer = 0
    const ask = () => {
      fetchReferences(after.target)
        .then((now) => {
          if (!live) return
          setReferences(now)
          if (now.indexing) {
            wasIndexing.current = true
            timer = window.setTimeout(ask, 1500)
          } else if (wasIndexing.current) {
            wasIndexing.current = false
            reload()
          }
        })
        .catch(() => {})
    }
    ask()
    return () => {
      live = false
      window.clearTimeout(timer)
    }
  }, [after, reloads, indexAsked, reload])

  // The address keeps the page and the comparison.
  useEffect(() => {
    writeTarget(target)
    const wanted = changesOn ? '#map/changes' : '#map'
    if (window.location.hash !== wanted) {
      window.history.replaceState(null, '', window.location.pathname + window.location.search + wanted)
    }
  }, [target, changesOn])

  // Fetch the map of the level, and the change set drawn on it.
  useEffect(() => {
    let live = true
    const wanted = after.target
    Promise.all([fetchMap(level, wanted), wanted ? fetchChanges(wanted) : Promise.resolve(null)])
      .then(([data, changes]) => {
        if (live) setLoaded({ data, changes, target: wanted })
      })
      .catch((problem: unknown) => {
        if (!live) return
        setError(problem instanceof ApiError || problem instanceof Error ? problem.message : String(problem))
        setBusy(false)
      })
    return () => {
      live = false
    }
  }, [level, reloads, after])

  // Lay it out: all of it, or only what the change is about.
  useEffect(() => {
    if (!loaded) return
    let live = true
    const data = isolated === 'all' || !loaded.data.overlay ? loaded.data : isolate(loaded.data, isolated)
    layoutMap(data, engine, cache, whole)
      .then((layout) => {
        if (!live) return
        const next = buildScene(data, layout)
        const overlay = loaded.data.overlay
        setView({
          scene: next,
          diff: overlay && loaded.changes ? buildDiff(next, overlay, loaded.changes.changes) : null,
          target: loaded.target,
        })
        setError(null)
        setBusy(false)
      })
      .catch((problem: unknown) => {
        if (!live) return
        setError(problem instanceof Error ? problem.message : String(problem))
        setBusy(false)
      })
    return () => {
      live = false
    }
  }, [loaded, isolated, whole, engine, cache])

  // A new map has arrived: put the view where it was asked to be, and keep
  // the selection on the same thing, or on what holds it.
  useLayoutEffect(() => {
    const handle = canvas.current
    if (!scene || !handle) return
    const whole = { x: 0, y: 0, width: scene.layout.width, height: scene.layout.height }
    const fits = fitRect(whole, handle.screen())
    const wanted = arrival.current
    arrival.current = { kind: 'stay' }
    // A card about a box of the old map says nothing about this one.
    window.clearTimeout(hoverTimer.current)
    setHover(null)
    if (wanted.kind === 'fit') handle.look(fits)
    else if (wanted.kind === 'show') {
      handle.look(fits)
      if (scene.boxes[wanted.id]) {
        setSelected(wanted.id)
        handle.show(wanted.id)
      }
    } else if (wanted.kind === 'follow') {
      const anchor = wanted.chain.find((id) => scene.boxes[id])
      if (anchor) {
        handle.look(
          follow(wanted.camera, wanted.rects[anchor], scene.boxes[anchor], {
            least: fits.scale * 0.6,
            most: 1.6,
          }),
        )
      } else handle.look(fits)
    }
    setSelected((id) => {
      if (id === null || scene.boxes[id]) return id
      return selectionChain.current.find((other) => scene.boxes[other]) ?? null
    })
  }, [scene])

  // The selected box and the boxes around it, to find it again on another level.
  useEffect(() => {
    if (scene && selected && scene.boxes[selected]) {
      selectionChain.current = [selected, ...ancestorsOf(scene, selected).reverse()]
    } else if (selected === null) selectionChain.current = []
  }, [scene, selected])

  // Who uses the selected box, and what it uses.
  useEffect(() => {
    if (selected === null) return
    let live = true
    nodeInfo(selected, level)
      .then((found) => {
        if (live) setInfo(found)
      })
      .catch(() => {})
    return () => {
      live = false
    }
  }, [selected, level, nodeInfo, reloads])
  const shownInfo = info && info.id === selected ? info : null

  const highlight = useMemo((): Highlight | null => {
    if (!scene || selected === null || !scene.boxes[selected]) return null
    const near = new Set<string>()
    const links = new Map<string, Link>()
    const join = (use: Use, direction: Link['direction']) => {
      const points =
        direction === 'in' ? connect(scene, use.at, selected) : connect(scene, selected, use.at)
      if (!points) return
      near.add(use.at)
      links.set(`${direction}:${use.at}`, { key: `${direction}:${use.at}`, points, direction })
    }
    if (shownInfo) {
      for (const use of shownInfo.callers) join(use, 'in')
      for (const use of shownInfo.dependencies) join(use, 'out')
    }
    const around = new Set<string>()
    for (const id of [selected, ...near]) for (const outer of ancestorsOf(scene, id)) around.add(outer)
    return { selected, near, around, links: [...links.values()] }
  }, [scene, selected, shownInfo])

  // The changes to read, in the order chosen.
  const order = useMemo(() => {
    const list = diff?.order ?? []
    return callerFirst ? [...list].reverse() : list
  }, [diff, callerFirst])

  /** Selects a box. `show` brings it into view; `record` adds it to the history. */
  const select = useCallback(
    (id: string | null, options: { show?: boolean; record?: boolean; change?: number } = {}) => {
      setSelected(id)
      setFocus(null)
      setHover(null)
      // The change drawn in the box becomes the one being read: its own,
      // or the first to read of those this level draws in it.
      if (options.change !== undefined) setCurrent(options.change)
      else if (id !== null && diff) {
        const own = diff.marks[id]?.own
        setCurrent(own ?? order.find((position) => diff.boxOf.get(position) === id) ?? null)
      } else setCurrent(null)
      if (id === null) return
      if (options.show) canvas.current?.show(id)
      if (options.record !== false) {
        setHistory((past) => {
          const last = past.stops[past.at]
          if (last && last.id === id && last.level === level) return past
          const stops = [...past.stops.slice(0, past.at + 1), { level, id }]
          return { stops, at: stops.length - 1 }
        })
      }
    },
    [level, diff, order],
  )

  /** Goes to a change: selects its box and brings it into view. */
  const pick = useCallback(
    (position: number) => {
      const id = diff?.boxOf.get(position)
      if (id) return select(id, { show: true, change: position })
      // A change with no box (a test): there is nothing to go to on the
      // map, but its before and after can be read.
      setSelected(null)
      setFocus(null)
      setCurrent(position)
    },
    [diff, select],
  )
  const stepChange = useCallback(
    (by: number) => {
      if (order.length === 0) return
      const at = current === null ? -1 : order.indexOf(current)
      const wanted = at === -1 ? (by > 0 ? 0 : order.length - 1) : at + by
      pick(order[Math.min(order.length - 1, Math.max(0, wanted))])
    },
    [order, current, pick],
  )

  const review = useCallback(
    (position: number, done: boolean) => {
      if (!diff) return
      saveReviewed(keysToMark(diff.changes, position), done)
        .then((keys) => setReviewed(new Set(keys)))
        .catch((problem: unknown) => setError(problem instanceof Error ? problem.message : String(problem)))
    },
    [diff],
  )

  // The before and after of the change being read.
  const viewTarget = view?.target ?? null
  useEffect(() => {
    if (current === null || !diff || !viewTarget) return
    let live = true
    fetchDetail(viewTarget, current, changeId(diff.changes[current]))
      .then((found) => {
        if (live) setDetail({ of: current, detail: found })
      })
      .catch(() => {})
    return () => {
      live = false
    }
  }, [current, diff, viewTarget])

  const layer = useMemo((): DiffLayer | null => {
    if (!diff) return null
    const at = current === null ? undefined : diff.boxOf.get(current)
    const pane = at && at === selected && detail?.of === current ? { id: at, detail: detail.detail } : null
    return { diff, reviewed, after, before, pane }
  }, [diff, reviewed, after, before, current, selected, detail])

  /** Goes to another level, keeping what the reader is looking at in place. */
  const changeLevel = useCallback(
    (next: Level, under?: string | null) => {
      const handle = canvas.current
      if (next === level || !scene || !handle) return
      const camera = handle.camera()
      const onScreen = (id: string) => {
        const box = scene.boxes[id]
        const corner = toScreen(camera, box.x + box.width / 2, box.y + box.height / 2)
        const size = handle.screen()
        return corner.x > 0 && corner.y > 0 && corner.x < size.width && corner.y < size.height
      }
      const anchor =
        under ?? (selected && scene.boxes[selected] && onScreen(selected) ? selected : handle.centre())
      const chain = anchor ? [anchor, ...ancestorsOf(scene, anchor).reverse()] : [scene.layout.root]
      arrival.current = {
        kind: 'follow',
        chain,
        rects: Object.fromEntries(chain.map((id) => [id, scene.boxes[id]])),
        camera,
      }
      setBusy(true)
      setLevel(next)
    },
    [level, scene, selected],
  )

  /** Zooming far in on something with more inside opens the next level. */
  const onWish = useCallback(
    (direction: 'deeper' | 'shallower', under: string | null) => {
      if (!auto || !scene || performance.now() - wished.current < 900) return
      const at = LEVELS.findIndex((each) => each.level === level)
      const next = LEVELS[at + (direction === 'deeper' ? 1 : -1)]
      if (!next) return
      if (direction === 'deeper') {
        // Only on a box that shows no more than its title at this level.
        const kind = under ? scene.boxes[under]?.detail : null
        if (kind !== 'title' && kind !== 'pill') return
      }
      wished.current = performance.now()
      changeLevel(next.level, under)
    },
    [auto, scene, level, changeLevel],
  )

  const goTo = useCallback(
    (stop: Stop) => {
      if (stop.level === level) return select(stop.id, { show: true, record: false })
      arrival.current = { kind: 'show', id: stop.id }
      setBusy(true)
      setLevel(stop.level)
    },
    [level, select],
  )
  const step = useCallback(
    (by: number) => {
      const stop = history.stops[history.at + by]
      if (!stop) return
      setHistory((past) => ({ ...past, at: past.at + by }))
      goTo(stop)
    },
    [history, goTo],
  )

  const choose = (hit: Hit) => {
    setSearching(false)
    if (scene?.boxes[hit.at]) return select(hit.at, { show: true })
    // Not a box at this level: go to the first level that shows it.
    setHistory((past) => {
      const stops = [...past.stops.slice(0, past.at + 1), { level: hit.level, id: hit.at }]
      return { stops, at: stops.length - 1 }
    })
    goTo({ level: hit.level, id: hit.at })
  }

  const jump = (use: Use) => {
    if (scene?.boxes[use.at]) select(use.at, { show: true })
  }

  /** Selects a scope, or the nearest box around it that this level shows. */
  const showScope = (id: string) => {
    if (!scene) return
    if (scene.boxes[id]) return select(id, { show: true })
    nodeInfo(id, level)
      .then((found) => {
        const around = found.ancestors.map((crumb) => crumb.id).reverse()
        const outer = around.find((each) => scene.boxes[each])
        if (outer) select(outer, { show: true })
      })
      .catch(() => {})
  }

  // The hover card appears once the pointer rests on a box.
  const onHover = useCallback(
    (id: string | null, at: { x: number; y: number }) => {
      window.clearTimeout(hoverTimer.current)
      if (id === null) {
        hoverTimer.current = window.setTimeout(() => setHover(null), 300)
        return
      }
      hoverTimer.current = window.setTimeout(() => {
        const kind = scene?.boxes[id]?.detail
        if (id === scene?.layout.root || kind === 'pill') return setHover(null)
        setHover((shown) => (shown?.id === id ? shown : { id, x: at.x, y: at.y }))
      }, 550)
    },
    [scene],
  )
  useEffect(() => {
    if (!hover) return
    let live = true
    nodeInfo(hover.id, level)
      .then((found) => {
        if (live) setHoverInfo(found)
      })
      .catch(() => {})
    return () => {
      live = false
    }
  }, [hover, level, nodeInfo])

  const index = () => {
    startIndexing()
      .then(() => setIndexAsked((count) => count + 1))
      .catch((problem: unknown) => setError(problem instanceof Error ? problem.message : String(problem)))
  }

  /** Another comparison, or none: the map is drawn afresh. */
  const compare = (on: boolean, next: Target = target) => {
    arrival.current = { kind: 'fit' }
    setBusy(true)
    setSelected(null)
    setCurrent(null)
    setDetail(null)
    setChangesOn(on)
    setTarget(next)
  }
  /** Opens the rest of a code box's text, or closes it; the box stays where it is. */
  const more = (id: string) => {
    const handle = canvas.current
    if (!scene || !handle || !scene.boxes[id]) return
    const chain = [id, ...ancestorsOf(scene, id).reverse()]
    arrival.current = {
      kind: 'follow',
      chain,
      rects: Object.fromEntries(chain.map((each) => [each, scene.boxes[each]])),
      camera: handle.camera(),
    }
    setBusy(true)
    setWhole((before) => {
      const next = new Set(before)
      if (!next.delete(id)) next.add(id)
      return next
    })
  }
  const cut = (next: Isolated) => {
    arrival.current = { kind: 'fit' }
    setBusy(true)
    setIsolated(next)
  }

  useEffect(() => {
    const keyDown = (event: KeyboardEvent) => {
      const typing =
        event.target instanceof HTMLInputElement ||
        event.target instanceof HTMLTextAreaElement ||
        event.target instanceof HTMLSelectElement
      if ((event.key === 'k' && (event.metaKey || event.ctrlKey)) || (event.key === '/' && !typing)) {
        event.preventDefault()
        setSearching(true)
        return
      }
      if (typing || event.metaKey || event.ctrlKey || event.altKey) return
      const handle = canvas.current
      if (event.key === 'Escape') {
        if (trace) setTrace(null)
        else if (hover) setHover(null)
        else select(null)
      } else if (trace || searching || !handle) return
      else if (event.key >= '1' && event.key <= '4') changeLevel(LEVELS[Number(event.key) - 1].level)
      else if (event.key === '0' || event.key === 'f') handle.fit()
      else if (event.key === '+' || event.key === '=') handle.zoom(1.4)
      else if (event.key === '-') handle.zoom(1 / 1.4)
      else if (event.key === '[') step(-1)
      else if (event.key === ']') step(1)
      else if (event.key === 't' && selected) setTrace(selected)
      else if (event.key === 'd') setDock((shown) => !shown)
      else if (event.key === 'n' || event.key === 'j') stepChange(1)
      else if (event.key === 'p' || event.key === 'k') stepChange(-1)
      else if (event.key === 'r' && current !== null && diff) {
        review(current, !reviewed.has(reviewKey(diff.changes[current])))
      } else if (event.key === 'ArrowLeft') handle.pan(-120, 0)
      else if (event.key === 'ArrowRight') handle.pan(120, 0)
      else if (event.key === 'ArrowUp') handle.pan(0, -120)
      else if (event.key === 'ArrowDown') handle.pan(0, 120)
      else return
      event.preventDefault()
    }
    window.addEventListener('keydown', keyDown)
    return () => window.removeEventListener('keydown', keyDown)
  }, [trace, hover, searching, selected, current, diff, reviewed, select, changeLevel, step, stepChange, review])

  const crumbs = scene && selected && scene.boxes[selected] ? [...ancestorsOf(scene, selected), selected] : []
  const total = scene ? Object.keys(scene.boxes).length : 0
  const currentChange =
    diff && current !== null && selected !== null && diff.boxOf.get(current) === selected
      ? diff.changes[current]
      : null

  return (
    <div className="map-view">
      <header className="map-bar">
        <a href="#" className="map-link">Changes</a>
        <strong>Map</strong>
        <div className="map-levels" role="group" aria-label="Level of detail">
          {LEVELS.map((each, position) => (
            <button
              key={each.level}
              type="button"
              className={each.level === level ? 'is-current' : ''}
              aria-pressed={each.level === level}
              title={`${each.shows} (${position + 1})`}
              onClick={() => changeLevel(each.level)}
            >
              {each.name}
            </button>
          ))}
        </div>
        <label title="Zooming far in on a box opens the next level; zooming far out goes back">
          <input type="checkbox" checked={auto} onChange={(event) => setAuto(event.target.checked)} /> Level by zoom
        </label>
        <div className="map-zoom" role="group" aria-label="Zoom">
          <button type="button" onClick={() => canvas.current?.zoom(1 / 1.4)} title="Zoom out (-)" aria-label="Zoom out">−</button>
          <button type="button" onClick={() => canvas.current?.fit()} title="Show the whole map (0)">Fit</button>
          <button type="button" onClick={() => canvas.current?.zoom(1.4)} title="Zoom in (+)" aria-label="Zoom in">+</button>
        </div>
        <div className="map-zoom" role="group" aria-label="History">
          <button type="button" disabled={history.at <= 0} onClick={() => step(-1)} title="Back ([)" aria-label="Back">←</button>
          <button type="button" disabled={history.at >= history.stops.length - 1} onClick={() => step(1)} title="Forward (])" aria-label="Forward">→</button>
        </div>
        <button type="button" onClick={() => setSearching(true)} title="Find a scope (/)">Search /</button>
        <button type="button" onClick={reload} title="Read the files again">Reload</button>
        {references && (
          <span className="map-refs" data-testid="map-refs" title={REFERENCES[references.by].means}>
            References:{' '}
            {references.indexing
              ? `indexing… ${references.indexing.step.replace(/^running /, '').replace(/ in \.$/, '')} (${references.indexing.seconds} s)`
              : REFERENCES[references.by].name}
            {references.can_index && (
              <button type="button" onClick={index} title="Run the language's own indexer, to also find methods called on a value. It can take minutes.">
                Index
              </button>
            )}
            {!references.indexing && references.notes.map((note) => (
              <span key={note} className="map-refs-note">{note}</span>
            ))}
          </span>
        )}
        <span
          className="map-stats"
          data-testid="map-stats"
          title={
            scene
              ? `Laid out in ${Math.round(scene.layout.stats.milliseconds)} ms: ${scene.layout.stats.laidOut} of ${scene.layout.stats.containers} containers, the rest reused`
              : undefined
          }
        >
          {busy ? 'Laying out…' : `${drawnCount} of ${total} boxes drawn`}
        </span>
      </header>
      <div className="map-bar map-bar-changes">
        <label title="Draw a comparison on the map">
          <input type="checkbox" checked={changesOn} onChange={(event) => compare(event.target.checked)} /> Changes
        </label>
        {changesOn && (
          <>
            <TargetPicker meta={meta} target={target} onTarget={(next) => compare(true, next)} />
            <label>
              <span className="field-label">Show</span>
              <select value={isolated} onChange={(event) => cut(event.target.value as Isolated)} aria-label="How much of the map">
                <option value="all">The whole map</option>
                <option value="changed">Only what changed</option>
                <option value="near">What changed and what touches it</option>
              </select>
            </label>
            <div className="map-zoom" role="group" aria-label="Step through the changes">
              <button type="button" disabled={order.length === 0} onClick={() => stepChange(-1)} title="Previous change (P)">Previous</button>
              <button type="button" disabled={order.length === 0} onClick={() => stepChange(1)} title="Next change (N)">Next</button>
            </div>
            <span className="map-legend" aria-label="Colours">
              <span className="t-added">added</span>
              <span className="t-modified">modified</span>
              <span className="t-moved">moved</span>
              <span className="t-removed">removed</span>
              <span className="is-impacted">affected</span>
            </span>
          </>
        )}
      </div>
      <nav className="map-crumbs" aria-label="Where the selection is">
        {crumbs.length === 0 && <span className="map-hint">Click a box's name to see what it touches; double-click it, or press T, to trace it. Drag to move, pinch or Ctrl-scroll to zoom.</span>}
        {crumbs.map((id, position) => (
          <span key={id}>
            {position > 0 && <span className="map-crumb-gap">/</span>}
            <button type="button" className={id === selected ? 'is-current' : ''} onClick={() => select(id, { show: true })}>
              {scene?.nodes[id]?.label}
            </button>
          </span>
        ))}
      </nav>
      {error && <p className="map-error">{error}</p>}
      <div className="map-body">
        {diff && (
          <ChangeList
            diff={diff}
            order={order}
            reviewed={reviewed}
            current={current}
            callerFirst={callerFirst}
            onFlip={() => setCallerFirst((flipped) => !flipped)}
            onPick={pick}
            onReview={review}
          />
        )}
        <div className="map-middle">
          {scene && (
            <MapCanvas
              ref={canvas}
              scene={scene}
              highlight={highlight}
              layer={layer}
              source={NOW}
              opened={whole}
              onMore={more}
              onSelect={select}
              onHover={onHover}
              onWish={onWish}
              onDrawn={setDrawnCount}
            onOpen={setTrace}
            />
          )}
          {diff && current !== null && dock && (
            <section className="mdock" aria-label="The change being read">
              <button type="button" className="mdock-close" onClick={() => setDock(false)} title="Hide (D)">
                Hide
              </button>
              <DiffView
                change={diff.changes[current]}
                detail={detail?.of === current ? detail.detail : null}
                error={null}
                onOpenCounterpart={(id) => {
                  const other = diff.changes.findIndex((each) => changeId(each) === id)
                  if (other !== -1) pick(other)
                }}
              />
            </section>
          )}
          {diff && current !== null && !dock && (
            <button type="button" className="mdock-open" onClick={() => setDock(true)} title="Show (D)">
              Show the before and after
            </button>
          )}
        </div>
        {shownInfo && (
          <Inspector
            info={shownInfo}
            change={
              currentChange && current !== null
                ? {
                    change: currentChange,
                    reviewed: reviewed.has(reviewKey(currentChange)),
                    onReview: (done) => review(current, done),
                  }
                : null
            }
            causes={(selected !== null && diff?.impacted[selected]) || []}
            focus={focus}
            onJump={jump}
            onTrace={() => setTrace(shownInfo.id)}
            onShow={() => canvas.current?.show(shownInfo.id)}
            onClose={() => select(null)}
          />
        )}
        {trace && (
          <Trace
            id={trace}
            source={after}
            onOpen={setTrace}
            onClose={() => setTrace(null)}
            onShowOnMap={(id) => {
              setTrace(null)
              showScope(id)
            }}
          />
        )}
      </div>
      {hover && !trace && (
        <HoverCard
          hover={hover}
          info={hoverInfo && hoverInfo.id === hover.id ? hoverInfo : null}
          onKeep={() => window.clearTimeout(hoverTimer.current)}
          onLeave={() => setHover(null)}
          onPick={(action) => {
            const id = hover.id
            if (action === 'open') {
              setHover(null)
              return setTrace(id)
            }
            select(id)
            setFocus(action)
          }}
        />
      )}
      {searching && <SearchPalette onChoose={choose} onClose={() => setSearching(false)} />}
    </div>
  )
}
