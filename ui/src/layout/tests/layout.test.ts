import { describe, expect, test } from 'vitest'
import { boxProblems, routeProblems } from '../check.ts'
import { inlineEngine } from '../engine-inline.ts'
import { LayoutCache, absolute, layoutMap } from '../layout.ts'
import { SIZE, codeLines, detailOf, sizeOf, titleOf, titleWidth } from '../sizes.ts'
import type { MapLayout } from '../types.ts'
import { container, map, nested, node, tenFiles, wire } from './fixtures.ts'

const engine = inlineEngine()

/** A layout without the numbers that change from run to run. */
function shape(layout: MapLayout): string {
  return JSON.stringify({ ...layout, stats: null })
}

describe('sizes', () => {
  test('a box is sized by what it shows, without drawing it', () => {
    const fn = node('f.py::run()', 'f.py', 'function', { label: 'run', lines: 10, columns: 50 })
    expect(detailOf(fn, 'module', false)).toBe('title')
    expect(detailOf(fn, 'code', false)).toBe('code')
    expect(detailOf(fn, 'code', true)).toBe('container')

    // The title says `function run`: the language's word, then the name.
    expect(titleOf(fn)).toBe('function run')
    expect(sizeOf(fn, 'title')).toEqual({ width: titleWidth('function run'), height: SIZE.title })
    expect(titleOf(node('f.py', '.', 'file'))).toBe('f.py')
    expect(sizeOf(fn, 'code')).toEqual({
      width: Math.ceil(50 * SIZE.codeChar + SIZE.gap),
      height: SIZE.title + 10 * SIZE.codeLine + SIZE.gap / 2,
    })
  })

  test('long code and long names are cut', () => {
    const huge = node('f.py::huge()', 'f.py', 'function', { lines: 900, columns: 400 })
    expect(sizeOf(huge, 'code')).toEqual({
      width: Math.ceil(SIZE.maxCodeColumns * SIZE.codeChar + SIZE.gap),
      height: SIZE.title + SIZE.maxCodeLines * SIZE.codeLine + SIZE.gap / 2,
    })
    expect(titleWidth('x'.repeat(500))).toBe(titleWidth('x'.repeat(SIZE.maxLabel)))
  })

  test('fields, variants and imports are rows; files are never code', () => {
    expect(detailOf(node('f.py::C::x', 'f.py::C', 'field'), 'code', false)).toBe('row')
    expect(detailOf(node('f.py::{import.1}', 'f.py', 'import'), 'code', false)).toBe('row')
    expect(detailOf(node('empty.py', '.', 'file'), 'code', false)).toBe('title')
  })
})

describe('a long block of code', () => {
  const long = node('f.py::long()', 'f.py', 'function', { lines: 100, columns: 50 })

  test('is cut, with one line kept to open the rest', () => {
    expect(codeLines(long, false)).toEqual({ shown: SIZE.maxCodeLines - 1, hidden: 101 - SIZE.maxCodeLines, foot: true })
    expect(sizeOf(long, 'code').height).toBe(SIZE.title + SIZE.maxCodeLines * SIZE.codeLine + SIZE.gap / 2)
    // A short one has no such line.
    expect(codeLines(node('f.py::s()', 'f.py', 'function', { lines: 10 }), false)).toEqual({ shown: 10, hidden: 0, foot: false })
  })

  test('opened whole, it shows every line and a line to close it', () => {
    expect(codeLines(long, true)).toEqual({ shown: 100, hidden: 0, foot: true })
    expect(sizeOf(long, 'code', true).height).toBe(SIZE.title + 101 * SIZE.codeLine + SIZE.gap / 2)
  })

  test('opening one lays out only the containers around it', async () => {
    const data = nested('code')
    data.nodes.find((each) => each.id === 'app/model.py::Shape::describe()')!.lines = 100
    const cache = new LayoutCache()
    const cut = await layoutMap(data, engine, cache)
    const whole = await layoutMap(data, engine, cache, new Set(['app/model.py::Shape::describe()']))
    expect(whole.boxes['app/model.py::Shape::describe()'].height).toBeGreaterThan(
      cut.boxes['app/model.py::Shape::describe()'].height,
    )
    expect(whole.stats.laidOut).toBe(4)
    expect(boxProblems(whole)).toEqual([])
    expect(whole.boxes['lib/util.py']).toMatchObject({ width: cut.boxes['lib/util.py'].width, height: cut.boxes['lib/util.py'].height })
  })
})

describe('one container', () => {
  test('ten files are laid out with nothing overlapping', async () => {
    const layout = await layoutMap(tenFiles(), engine)
    expect(Object.keys(layout.boxes)).toHaveLength(11)
    expect(boxProblems(layout)).toEqual([])
    expect(routeProblems(layout)).toEqual([])
    expect(layout.stats).toMatchObject({ containers: 1, laidOut: 1, engineCalls: 1 })
  })

  test('wires run left to right, from what uses to what is used', async () => {
    const layout = await layoutMap(tenFiles(), engine)
    for (const wire of layout.containers['.'].wires) {
      const [from, to] = [layout.boxes[wire.from], layout.boxes[wire.to]]
      if (wire.kind === 'cycle') {
        // The wire that closes the loop is the one drawn right to left.
        expect(to.x + to.width).toBeLessThan(from.x)
        expect(wire.route![0].x).toBe(from.x)
        expect(wire.route!.at(-1)!.x).toBe(to.x + to.width)
      } else {
        expect(from.x + from.width).toBeLessThan(to.x)
      }
      // A wire attaches at title height.
      expect(wire.route![0].y).toBe(from.y + SIZE.title / 2)
    }
  })

  test('children with no wires wrap into rows, in source order, under the rest', async () => {
    const layout = await layoutMap(tenFiles(), engine)
    const [alone, apart] = [layout.boxes['alone.py'], layout.boxes['apart.py']]
    const lowestWired = Math.max(
      ...['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'].map((name) => {
        const box = layout.boxes[`${name}.py`]
        return box.y + box.height
      }),
    )
    expect(alone.y).toBeGreaterThan(lowestWired)
    expect(apart.y).toBe(alone.y)
    expect(apart.x).toBe(alone.x + alone.width + SIZE.gap)
  })

  test('many children with no wires make a block, not one long line', async () => {
    const files = Array.from({ length: 40 }, (_, i) => node(`f${String(i).padStart(2, '0')}.py`, '.', 'file'))
    const layout = await layoutMap(map('package', [node('.', null, 'directory'), ...files]), engine)
    expect(boxProblems(layout)).toEqual([])
    const shelves = new Set(files.map((file) => layout.boxes[file.id].y))
    expect(shelves.size).toBeGreaterThan(3)
    expect(layout.width / layout.height).toBeGreaterThan(0.8)
    expect(layout.width / layout.height).toBeLessThan(4)
    // No engine is needed when nothing is wired.
    expect(layout.stats.engineCalls).toBe(0)
  })
})

describe('the top level', () => {
  test('files are small marks packed in their directory', async () => {
    const files = Array.from({ length: 30 }, (_, i) =>
      node(`src/f${String(i).padStart(2, '0')}.rs`, 'src', 'file', { minor: true }),
    )
    const data = map(
      'architecture',
      [node('.', null, 'directory'), node('src', '.', 'directory'), ...files, node('docs', '.', 'directory')],
      { '.': container([wire('src', 'docs')]) },
    )
    const layout = await layoutMap(data, engine)
    expect(boxProblems(layout)).toEqual([])
    expect(layout.boxes['src/f00.rs']).toMatchObject({
      detail: 'pill',
      width: SIZE.pillWidth,
      height: SIZE.pillHeight,
    })
    // Thirty files take far less room than thirty titles would.
    expect(layout.boxes.src.width * layout.boxes.src.height).toBeLessThan(30 * SIZE.minWidth * SIZE.title)
    const rows = new Set(files.map((file) => layout.boxes[file.id].y))
    expect(rows.size).toBeGreaterThan(1)
  })
})

describe('nested containers', () => {
  test('every box is inside its parent and beside its siblings', async () => {
    for (const level of ['module', 'code'] as const) {
      const layout = await layoutMap(nested(level), engine)
      expect(boxProblems(layout)).toEqual([])
      expect(layout.boxes['.']).toMatchObject({ x: 0, y: 0, width: layout.width, height: layout.height })
    }
  })

  test('a container is as big as what it holds, and its parent as big as that', async () => {
    const small = await layoutMap(nested('module'), engine)
    const big = await layoutMap(nested('code'), engine)
    // At code level a method shows its text, so everything around it grows.
    for (const id of ['app/model.py::Shape', 'app/model.py', 'app', '.']) {
      expect(big.boxes[id].width).toBeGreaterThan(small.boxes[id].width)
      expect(big.boxes[id].height).toBeGreaterThan(small.boxes[id].height)
    }
  })

  test('fields are stacked in one column, above the methods', async () => {
    const layout = await layoutMap(nested(), engine)
    const [name, sides] = [
      layout.boxes['app/model.py::Shape::name'],
      layout.boxes['app/model.py::Shape::sides'],
    ]
    expect(name.detail).toBe('row')
    expect(sides.x).toBe(name.x)
    expect(sides.y).toBe(name.y + SIZE.row)
    expect(sides.width).toBe(name.width)
    for (const method of ['area()', 'scale()', 'describe()']) {
      expect(layout.boxes[`app/model.py::Shape::${method}`].y).toBeGreaterThan(sides.y + sides.height)
    }
  })

  test('no wire runs through a box it does not belong to', async () => {
    for (const level of ['module', 'code'] as const) {
      const layout = await layoutMap(nested(level), engine)
      expect(routeProblems(layout)).toEqual([])
    }
  })

  test('a container is entered on its left and left on its right, at title height', async () => {
    const layout = await layoutMap(nested(), engine)
    const shape = layout.containers['app/model.py::Shape']
    const outer = layout.boxes['app/model.py::Shape']
    // Into a method, into a field; out of a method, out of a field.
    expect(Object.keys(shape.entries).sort()).toEqual([
      'app/model.py::Shape::area()',
      'app/model.py::Shape::sides',
    ])
    for (const [child, route] of Object.entries(shape.entries)) {
      const box = layout.boxes[child]
      expect(route[0]).toEqual({ x: 0, y: SIZE.title / 2 })
      expect(route.at(-1)!.x).toBe(box.x)
      expect(route.at(-1)!.y).toBe(box.y + (box.detail === 'row' ? SIZE.row : SIZE.title) / 2)
    }
    for (const [child, route] of Object.entries(shape.exits)) {
      const box = layout.boxes[child]
      expect(route.at(-1)).toEqual({ x: outer.width, y: SIZE.title / 2 })
      expect(route[0].x).toBe(box.x + box.width)
    }
    // A child with no wires at all is reached along the gap above its row.
    expect(layout.containers['app/main.py'].exits['app/main.py::spare()']).toBeDefined()
    expect(layout.containers['lib/util.py'].entries['lib/util.py::log()']).toBeDefined()
  })

  test('a wire to a row has no route of its own', async () => {
    const layout = await layoutMap(nested(), engine)
    const wires = layout.containers['app/model.py::Shape'].wires
    expect(wires.find((wire) => wire.to.endsWith('::name'))!.route).toBeNull()
    expect(wires.find((wire) => wire.to.endsWith('::area()'))!.route).not.toBeNull()
  })

  test('absolute corners add up through the parents', async () => {
    const layout = await layoutMap(nested(), engine)
    const corners = absolute(layout)
    const id = 'app/model.py::Shape::area()'
    let [x, y] = [0, 0]
    for (let at: string | null = id; at !== null; at = layout.boxes[at].parent) {
      x += layout.boxes[at].x
      y += layout.boxes[at].y
    }
    expect(corners[id]).toMatchObject({ x, y })
  })
})

describe('stability', () => {
  test('twenty runs give the same coordinates', async () => {
    const first = shape(await layoutMap(nested('code'), engine))
    for (let run = 0; run < 19; run++) {
      expect(shape(await layoutMap(nested('code'), engine))).toBe(first)
    }
    const ten = shape(await layoutMap(tenFiles(), engine))
    for (let run = 0; run < 19; run++) {
      expect(shape(await layoutMap(tenFiles(), engine))).toBe(ten)
    }
  })

  test('with a cache, nothing is laid out twice', async () => {
    const cache = new LayoutCache()
    const first = await layoutMap(nested(), engine, cache)
    expect(first.stats.laidOut).toBe(first.stats.containers)
    const second = await layoutMap(nested(), engine, cache)
    expect(second.stats).toMatchObject({ laidOut: 0, engineCalls: 0 })
    expect(shape(second)).toBe(shape(first))
  })

  test('a change to one box lays out only the containers around it', async () => {
    const cache = new LayoutCache()
    const before = await layoutMap(nested('code'), engine, cache)

    const changed = nested('code')
    const scale = changed.nodes.find((each) => each.id === 'app/model.py::Shape::scale()')!
    scale.lines += 12
    const after = await layoutMap(changed, engine, cache)

    // The class, its file, its directory and the root. Nothing else.
    expect(after.stats.laidOut).toBe(4)
    for (const untouched of ['app/main.py', 'lib', 'lib/util.py']) {
      expect(after.containers[untouched]).toEqual(before.containers[untouched])
      expect(after.boxes[untouched]).toMatchObject({
        width: before.boxes[untouched].width,
        height: before.boxes[untouched].height,
      })
    }
    for (const child of Object.values(after.boxes)) {
      if (child.parent === 'lib/util.py' || child.parent === 'app/main.py') {
        expect(child).toEqual(before.boxes[child.id])
      }
    }
    expect(after.boxes['app/model.py::Shape'].height).toBeGreaterThan(
      before.boxes['app/model.py::Shape'].height,
    )
  })

  test('a new wire elsewhere does not disturb a container', async () => {
    const cache = new LayoutCache()
    const before = await layoutMap(nested(), engine, cache)
    const changed = nested()
    changed.containers['lib/util.py'] = container(
      [wire('lib/util.py::clamp()', 'lib/util.py::log()')],
      ['lib/util.py::clamp()', 'lib/util.py::log()'],
    )
    const after = await layoutMap(changed, engine, cache)
    expect(after.containers['app/model.py::Shape']).toEqual(before.containers['app/model.py::Shape'])
    expect(after.containers['app/main.py']).toEqual(before.containers['app/main.py'])
  })
})

describe('a comparison', () => {
  test('what was removed has a place, so both sides share one layout', async () => {
    const merged = nested()
    // `setup` was removed and `fresh` was added: the merged map has both.
    merged.nodes.find((each) => each.id === 'app/main.py::setup()')!.presence = 'removed'
    merged.nodes.splice(
      merged.nodes.findIndex((each) => each.id === 'app/main.py::spare()') + 1,
      0,
      node('app/main.py::fresh()', 'app/main.py', 'function', { presence: 'added' }),
    )
    merged.containers['app/main.py'].wires[0].presence = 'removed'
    merged.containers['app/main.py'].wires.push(
      wire('app/main.py::run()', 'app/main.py::fresh()', 'direct', 'added'),
    )

    const layout = await layoutMap(merged, engine)
    expect(boxProblems(layout)).toEqual([])
    expect(routeProblems(layout)).toEqual([])
    expect(layout.boxes['app/main.py::setup()'].presence).toBe('removed')
    expect(layout.boxes['app/main.py::fresh()'].presence).toBe('added')
    const wires = layout.containers['app/main.py'].wires
    expect(wires.map((each) => [each.to.split('::')[1], each.presence, each.route !== null])).toEqual([
      ['setup()', 'removed', true],
      ['fresh()', 'added', true],
    ])

    // Either side is this layout with some boxes not drawn: every box that
    // is on both sides is where it is, whichever side is shown.
    const before = Object.values(layout.boxes).filter((box) => box.presence !== 'added')
    const after = Object.values(layout.boxes).filter((box) => box.presence !== 'removed')
    expect(before.length).toBe(after.length)
    expect(before.some((box) => box.id.endsWith('setup()'))).toBe(true)
    expect(after.some((box) => box.id.endsWith('setup()'))).toBe(false)
  })
})
