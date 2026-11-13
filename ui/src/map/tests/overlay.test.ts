import { beforeAll, describe, expect, test } from 'vitest'
import type { Change } from '../../api'
import { inlineEngine } from '../../layout/engine-inline.ts'
import { layoutMap } from '../../layout/layout.ts'
import { nested, node } from '../../layout/tests/fixtures.ts'
import type { MapData, Overlay } from '../../layout/types.ts'
import { reviewKey } from '../../model'
import { buildDiff, chipOf, isolate, reviewedIn, type Diff } from '../overlay.ts'
import { buildScene } from '../scene.ts'

function change(state: Change['state'], id: string, more: Partial<Change> = {}): Change {
  const gone = state === 'removed' || state === 'moved_out'
  return {
    state,
    kinds: state === 'modified' ? ['body'] : [],
    kind: 'function',
    lang_kind: 'def',
    label: id.split('::').pop() ?? id,
    old_label: null,
    name: null,
    access: 'public',
    old_id: state === 'added' || state === 'moved_in' ? null : id,
    new_id: gone ? null : id,
    counterpart: null,
    old_path: null,
    new_path: null,
    own: { added: 3, removed: 1 },
    total: { added: 3, removed: 1 },
    moved_delta: null,
    fingerprint: `f-${id}`,
    parent: null,
    children: [],
    ...more,
  }
}

// `setup` was removed, `fresh` added, `run` and `area` modified, and
// `log` moved from `lib/util.py` to `app/main.py`.
const CHANGES: Change[] = [
  change('modified', 'app/main.py::run()'),
  change('removed', 'app/main.py::setup()'),
  change('added', 'app/main.py::fresh()'),
  change('modified', 'app/model.py::Shape::area()'),
  change('moved_out', 'lib/util.py::log()', { counterpart: 'app/main.py::log()' }),
  change('moved_in', 'app/main.py::log()', { counterpart: 'lib/util.py::log()' }),
]

function changedMap(level: 'module' | 'package'): MapData {
  const map = nested('module')
  map.nodes.find((each) => each.id === 'app/main.py::setup()')!.presence = 'removed'
  map.nodes.find((each) => each.id === 'lib/util.py::log()')!.presence = 'removed'
  const at = map.nodes.findIndex((each) => each.id === 'app/main.py::spare()') + 1
  map.nodes.splice(
    at,
    0,
    node('app/main.py::fresh()', 'app/main.py', 'function', { presence: 'added' }),
    node('app/main.py::log()', 'app/main.py', 'function', { presence: 'added' }),
  )
  const overlay: Overlay = {
    changes: {
      'app/main.py::run()': [0],
      'app/main.py::setup()': [1],
      'app/main.py::fresh()': [2],
      'app/model.py::Shape::area()': [3],
      'lib/util.py::log()': [4],
      'app/main.py::log()': [5],
    },
    near: ['app/model.py::Shape::scale()', 'lib/util.py::clamp()'],
    impacted: {
      'app/model.py::Shape::scale()': [{ at: 'app/model.py::Shape::area()', label: 'area()', reasons: ['signature changed'] }],
    },
    order: [3, 4, 5, 1, 2, 0],
  }
  if (level === 'package') {
    // Files are the boxes: each holds the changes of what is inside it.
    const files = new Set(['.', 'app', 'app/main.py', 'app/model.py', 'lib', 'lib/util.py'])
    map.nodes = map.nodes.filter((each) => files.has(each.id))
    for (const each of map.nodes) each.presence = 'both'
    map.containers = { '.': map.containers['.'], app: map.containers.app }
    overlay.changes = { 'app/main.py': [0, 1, 2, 5], 'app/model.py': [3], 'lib/util.py': [4] }
    overlay.near = []
    overlay.impacted = {}
  }
  return { ...map, level, overlay }
}

async function diffOf(map: MapData): Promise<Diff> {
  const scene = buildScene(map, await layoutMap(map, inlineEngine()))
  return buildDiff(scene, map.overlay!, CHANGES)
}

describe('a change on the map', () => {
  let diff: Diff
  beforeAll(async () => {
    diff = await diffOf(changedMap('module'))
  })

  test('each changed box is coloured by what happened to it', () => {
    const tone = (id: string) => diff.marks[id]?.tone
    expect(tone('app/main.py::run()')).toBe('modified')
    expect(tone('app/main.py::setup()')).toBe('removed')
    expect(tone('app/main.py::fresh()')).toBe('added')
    expect(tone('lib/util.py::log()')).toBe('moved')
    expect(tone('app/main.py::log()')).toBe('moved')
    // A box nothing happened to has no mark at all.
    expect(diff.marks['app/main.py::unused()']).toBeUndefined()
    expect(diff.marks['lib/util.py::clamp()']).toBeUndefined()
  })

  test('a container counts what changed inside it, and so does everything around it', () => {
    const file = diff.marks['app/main.py']
    // It holds changes but did not change as a whole.
    expect(file.tone).toBeNull()
    expect(file.counts).toEqual({ added: 1, removed: 1, modified: 1, moved: 1 })
    expect(chipOf(file)).toBe('+1 ~1 →1 −1')
    expect(diff.marks['app/model.py::Shape'].inside).toEqual([3])
    expect(diff.marks.app.inside).toEqual([0, 1, 2, 3, 5])
    expect(diff.marks['.'].inside).toHaveLength(6)
  })

  test('review marks roll up to containers', () => {
    const reviewed = new Set([reviewKey(CHANGES[0]), reviewKey(CHANGES[3])])
    expect(reviewedIn(diff, 'app/main.py', reviewed)).toEqual({ done: 1, of: 4 })
    expect(reviewedIn(diff, 'app', reviewed)).toEqual({ done: 2, of: 5 })
    expect(reviewedIn(diff, 'app/model.py::Shape::area()', reviewed)).toEqual({ done: 1, of: 1 })
    expect(reviewedIn(diff, 'lib/util.py::clamp()', reviewed)).toEqual({ done: 0, of: 0 })
  })

  test('a moved scope is joined from where it was to where it is', () => {
    expect(diff.moves).toEqual([{ from: 'lib/util.py::log()', to: 'app/main.py::log()' }])
  })

  test('a change to a test is listed, though it has no box', async () => {
    const map = changedMap('module')
    // Position 6 is a change to a test: in the order and in `tests`, in no box.
    map.overlay!.order = [...map.overlay!.order, 6]
    map.overlay!.tests = [6]
    const scene = buildScene(map, await layoutMap(map, inlineEngine()))
    const withTest = buildDiff(scene, map.overlay!, [...CHANGES, change('modified', 'tests/test_app.py::test_run()')])
    expect(withTest.order.at(-1)).toBe(6)
    expect(withTest.tests.has(6)).toBe(true)
    expect(withTest.boxOf.has(6)).toBe(false)
    // It counts for no box, so no container's chip includes it.
    expect(withTest.marks['.'].inside).not.toContain(6)
  })

  test('the reading order and the neighbours come through', () => {
    expect(diff.order).toEqual([3, 4, 5, 1, 2, 0])
    expect([...diff.near]).toEqual(['app/model.py::Shape::scale()', 'lib/util.py::clamp()'])
    expect(diff.impacted['app/model.py::Shape::scale()'][0].label).toBe('area()')
  })

  test('where files are the boxes, a file is marked by the changes in it', async () => {
    const files = await diffOf(changedMap('package'))
    expect(files.marks['app/main.py']).toMatchObject({ own: null, tone: 'modified' })
    expect(files.marks['app/main.py'].inside).toEqual([0, 1, 2, 5])
    expect(files.marks.app.tone).toBeNull()
    expect(files.boxOf.get(3)).toBe('app/model.py')
    // Both ends of the move are in different files: still one link.
    expect(files.moves).toEqual([{ from: 'lib/util.py', to: 'app/main.py' }])
  })

  test('what is inside a box that was added whole is added with it', async () => {
    const map = changedMap('module')
    map.nodes.push(
      node('new', '.', 'directory', { presence: 'added' }),
      node('new/mod.py', 'new', 'file', { presence: 'added' }),
      node('new/mod.py::start()', 'new/mod.py', 'function', { presence: 'added' }),
    )
    const whole = await diffOf(map)
    expect(whole.marks['new/mod.py::start()'].tone).toBe('added')
    expect(whole.marks.new.tone).toBe('added')
  })
})

describe('the map cut down to a change', () => {
  const ids = (map: MapData) => map.nodes.map((each) => each.id)

  test('changed boxes stay, with what is around them', () => {
    const cut = isolate(changedMap('module'), 'changed')
    expect(ids(cut)).toEqual([
      '.',
      'app',
      'app/main.py',
      'app/main.py::run()',
      'app/main.py::setup()',
      'app/main.py::fresh()',
      'app/main.py::log()',
      'app/model.py',
      'app/model.py::Shape',
      'app/model.py::Shape::area()',
      'lib',
      'lib/util.py',
      'lib/util.py::log()',
    ])
    // Wires are kept between boxes that are both still there.
    expect(cut.containers['app/main.py'].wires.map((wire) => wire.to)).toEqual(['app/main.py::setup()'])
    expect(cut.containers['app/model.py::Shape'].wires).toEqual([])
    expect(cut.containers['lib/util.py'].entries.map((door) => door.id)).toEqual(['lib/util.py::log()'])
  })

  test('with neighbours, the unchanged boxes that touch a change stay too', () => {
    const cut = isolate(changedMap('module'), 'near')
    expect(ids(cut)).toContain('app/model.py::Shape::scale()')
    expect(ids(cut)).toContain('lib/util.py::clamp()')
    expect(ids(cut)).not.toContain('app/main.py::unused()')
    expect(cut.containers['app/model.py::Shape'].wires.map((wire) => wire.from)).toEqual([
      'app/model.py::Shape::scale()',
    ])
  })

  test('a box with a change drawn in it does not bring its unchanged children', () => {
    // An import changed: at this level that is drawn in the file.
    const map = changedMap('module')
    map.overlay!.changes['lib/util.py'] = [4]
    delete map.overlay!.changes['lib/util.py::log()']
    map.nodes.find((each) => each.id === 'lib/util.py::log()')!.presence = 'both'
    const cut = isolate(map, 'changed')
    expect(ids(cut)).toContain('lib/util.py')
    expect(ids(cut)).not.toContain('lib/util.py::clamp()')
    expect(ids(cut)).not.toContain('lib/util.py::log()')
  })

  test('a map with no comparison is left as it is', () => {
    const plain = nested()
    expect(isolate(plain, 'changed')).toBe(plain)
  })

  test('the cut map lays out, smaller than the whole', async () => {
    const engine = inlineEngine()
    const whole = await layoutMap(changedMap('module'), engine)
    const cut = await layoutMap(isolate(changedMap('module'), 'changed'), engine)
    expect(Object.keys(cut.boxes).length).toBeLessThan(Object.keys(whole.boxes).length)
    expect(cut.width * cut.height).toBeLessThan(whole.width * whole.height)
  })
})
