import { beforeAll, describe, expect, test } from 'vitest'
import { inlineEngine } from '../../layout/engine-inline.ts'
import { layoutMap } from '../../layout/layout.ts'
import { SIZE } from '../../layout/sizes.ts'
import { nested } from '../../layout/tests/fixtures.ts'
import { covers, fitRect, follow, panBy, toScreen, viewOf, zoomAt } from '../camera.ts'
import { connect, doors } from '../paths.ts'
import { ancestorsOf, boxAt, buildScene, fileOf, type Scene } from '../scene.ts'
import { visibleIn } from '../visible.ts'

const screen = { width: 1200, height: 800 }
let scene: Scene
let codeScene: Scene

beforeAll(async () => {
  const engine = inlineEngine()
  scene = buildScene(nested('module'), await layoutMap(nested('module'), engine))
  codeScene = buildScene(nested('code'), await layoutMap(nested('code'), engine))
})

describe('the scene', () => {
  test('knows what is around a box and what is under a point', () => {
    const id = 'app/model.py::Shape::area()'
    expect(ancestorsOf(scene, id)).toEqual(['.', 'app', 'app/model.py', 'app/model.py::Shape'])
    expect(fileOf(scene, id)).toBe('app/model.py')
    expect(fileOf(scene, 'app')).toBeNull()

    const box = scene.boxes[id]
    expect(boxAt(scene, box.x + 2, box.y + 2)).toBe(id)
    // The gap beside it belongs to the class around it.
    expect(boxAt(scene, box.x - 2, box.y + 2)).toBe('app/model.py::Shape')
    expect(boxAt(scene, -5, -5)).toBeNull()
  })
})

describe('the camera', () => {
  test('fits a rectangle in the middle of the screen', () => {
    const camera = fitRect({ x: 100, y: 50, width: 2400, height: 600 }, screen, 0)
    expect(camera.scale).toBe(0.5)
    expect(toScreen(camera, 100, 50)).toEqual({ x: 0, y: 250 })
    // Never closer than asked, however small the rectangle.
    expect(fitRect({ x: 0, y: 0, width: 10, height: 10 }, screen).scale).toBe(1)
  })

  test('zooming keeps the point under the pointer where it is', () => {
    const before = { x: 300, y: 200, scale: 0.5 }
    const pointer = { x: 400, y: 300 }
    const after = zoomAt(before, 2, pointer)
    expect(after.scale).toBe(1)
    const world = { x: before.x + pointer.x / before.scale, y: before.y + pointer.y / before.scale }
    expect(toScreen(after, world.x, world.y)).toEqual(pointer)
  })

  test('panning moves by a distance on screen', () => {
    expect(panBy({ x: 0, y: 0, scale: 0.5 }, 100, -50)).toEqual({ x: 200, y: -100, scale: 0.5 })
    expect(viewOf({ x: 10, y: 20, scale: 2 }, screen)).toEqual({ x: 10, y: 20, width: 600, height: 400 })
  })

  test('going to another level keeps a box where it is on screen', () => {
    const camera = { x: 0, y: 0, scale: 1.5 }
    const before = scene.boxes['app/model.py']
    const after = codeScene.boxes['app/model.py']
    const next = follow(camera, before, after, { least: 0.01, most: 2 })
    const [was, is] = [toScreen(camera, before.x, before.y), toScreen(next, after.x, after.y)]
    expect(is.x).toBeCloseTo(was.x)
    expect(is.y).toBeCloseTo(was.y, 0)
    expect(after.width * next.scale).toBeCloseTo(before.width * camera.scale)
  })

  test('a drawing is reused until the view leaves it or the zoom changes much', () => {
    const camera = { x: 100, y: 100, scale: 1 }
    const drawnFor = { view: { x: -500, y: -300, width: 2400, height: 1600 }, scale: 1 }
    expect(covers(drawnFor, camera, screen)).toBe(true)
    expect(covers(drawnFor, panBy(camera, 900, 0), screen)).toBe(false)
    expect(covers(drawnFor, zoomAt(camera, 1.1, { x: 600, y: 400 }), screen)).toBe(true)
    expect(covers(drawnFor, zoomAt(camera, 1.4, { x: 600, y: 400 }), screen)).toBe(false)
  })
})

describe('what is drawn', () => {
  const all = (target: Scene) => ({ x: 0, y: 0, width: target.layout.width, height: target.layout.height })

  test('at full size everything is drawn, with titles in place', () => {
    const seen = visibleIn(scene, all(scene), 1)
    expect(seen.boxes).toHaveLength(Object.keys(scene.boxes).length)
    expect(seen.boxes.every((box) => box.label === 'title')).toBe(true)
    expect(seen.wires).toContainEqual({ id: 'app', counts: true })
  })

  test('far out, containers close and small boxes are left out', () => {
    const far = visibleIn(scene, all(scene), 0.04)
    const ids = far.boxes.map((box) => box.id)
    expect(ids).toContain('.')
    expect(ids).not.toContain('app/model.py::Shape::area()')
    expect(far.wires).toEqual([])
    // Nothing is drawn inside a container that is closed.
    for (const box of far.boxes) {
      const parent = scene.boxes[box.id].parent
      if (parent !== null) expect(far.boxes.find((other) => other.id === parent)!.open).toBe(true)
    }
  })

  test('in between, a title too small to read floats at a readable size', () => {
    const seen = visibleIn(scene, all(scene), 0.3)
    expect(seen.boxes.find((box) => box.id === 'app')!.label).toBe('float')
    // A row is too small to label at all.
    expect(seen.boxes.find((box) => box.id.endsWith('::name'))?.label ?? 'none').toBe('none')
  })

  test('only what is on screen is drawn', () => {
    const lib = scene.boxes.lib
    const seen = visibleIn(scene, { x: lib.x + 4, y: lib.y + 4, width: 30, height: 30 }, 1)
    const ids = seen.boxes.map((box) => box.id)
    expect(ids).toContain('lib')
    expect(ids).not.toContain('app/main.py')
  })

  test('source text is drawn only when a line is high enough to read', () => {
    const id = 'app/model.py::Shape::area()'
    const text = (scale: number) =>
      visibleIn(codeScene, all(codeScene), scale).boxes.find((box) => box.id === id)!.text
    expect(text(1)).toBe(true)
    expect(text(0.2)).toBe(false)
    expect(visibleIn(scene, all(scene), 1).boxes.every((box) => !box.text)).toBe(true)
  })
})

describe('a path between two boxes', () => {
  test('between siblings it is the wire the layout made', () => {
    const path = connect(scene, 'app/main.py::run()', 'app/main.py::setup()')!
    expect(path[0]).toEqual(doors(scene, 'app/main.py::run()').out)
    expect(path.at(-1)).toEqual(doors(scene, 'app/main.py::setup()').in)
  })

  test('across containers it leaves by the way out and arrives by the way in', () => {
    const [from, to] = ['app/main.py::run()', 'lib/util.py::clamp()']
    const path = connect(scene, from, to)!
    expect(path[0]).toEqual(doors(scene, from).out)
    expect(path.at(-1)).toEqual(doors(scene, to).in)
    // It passes through the doors of each container in between.
    for (const door of [
      doors(scene, 'app/main.py').out,
      doors(scene, 'app').out,
      doors(scene, 'lib').in,
      doors(scene, 'lib/util.py').in,
    ]) {
      expect(path).toContainEqual(door)
    }
    // Every piece is straight.
    for (let i = 1; i < path.length; i++) {
      expect(path[i].x === path[i - 1].x || path[i].y === path[i - 1].y).toBe(true)
    }
  })

  test('there is none between a box and what it holds', () => {
    expect(connect(scene, 'app', 'app/main.py::run()')).toBeNull()
    expect(connect(scene, 'app/main.py::run()', 'app/main.py')).toBeNull()
    expect(connect(scene, 'app', 'nowhere')).toBeNull()
  })

  test('where the layout made no piece, the doors are joined directly', () => {
    // `unused` has no way out of its file in the layout.
    const path = connect(scene, 'app/main.py::unused()', 'lib/util.py::log()')!
    expect(path[0]).toEqual(doors(scene, 'app/main.py::unused()').out)
    expect(path[1]).toEqual(doors(scene, 'app/main.py').out)
    expect(path.at(-1)!.y).toBe(scene.boxes['lib/util.py::log()'].y + SIZE.title / 2)
  })
})
