// Small maps written by hand, where every answer can be worked out.

import type { Level, MapContainer, MapData, MapNode, Presence, WireKind } from '../types.ts'

export function node(
  id: string,
  parent: string | null,
  kind: string,
  more: Partial<MapNode> = {},
): MapNode {
  const label = id === '.' ? 'repo' : (id.split(/::|\//).pop() ?? id)
  return {
    id,
    parent,
    kind,
    lang_kind: kind,
    label,
    line: 1,
    lines: 6,
    columns: 40,
    minor: false,
    presence: 'both',
    ...more,
  }
}

export function wire(
  from: string,
  to: string,
  kind: WireKind = 'direct',
  presence: Presence = 'both',
) {
  return { from, to, kind, count: 1, presence }
}

export function container(
  wires: ReturnType<typeof wire>[],
  entries: string[] = [],
  exits: string[] = [],
): MapContainer {
  return {
    wires,
    entries: entries.map((id) => ({ id })),
    exits: exits.map((id) => ({ id })),
  }
}

export function map(
  level: Level,
  nodes: MapNode[],
  containers: Record<string, MapContainer> = {},
): MapData {
  return { level, nodes, containers }
}

/** Ten files in one directory: a chain, a fan, a loop, and two on their own. */
export function tenFiles(): MapData {
  const files = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'alone', 'apart']
  return map(
    'package',
    [node('.', null, 'directory'), ...files.map((name) => node(`${name}.py`, '.', 'file'))],
    {
      '.': container([
        wire('a.py', 'b.py'),
        wire('a.py', 'c.py'),
        wire('b.py', 'd.py'),
        wire('c.py', 'd.py'),
        wire('d.py', 'e.py'),
        wire('e.py', 'f.py'),
        wire('f.py', 'g.py'),
        // `g` uses `e` back: the wire that closes the loop.
        wire('g.py', 'e.py', 'cycle'),
        wire('a.py', 'h.py'),
      ]),
    },
  )
}

/**
 * Two packages. `app` holds two files; one of them holds a class with
 * fields and methods. `lib` is used from `app`.
 */
export function nested(level: Level = 'module'): MapData {
  return map(
    level,
    [
      node('.', null, 'directory'),
      node('app', '.', 'directory'),
      node('app/main.py', 'app', 'file'),
      node('app/main.py::run()', 'app/main.py', 'function'),
      node('app/main.py::setup()', 'app/main.py', 'function'),
      node('app/main.py::unused()', 'app/main.py', 'function'),
      node('app/main.py::spare()', 'app/main.py', 'function'),
      node('app/model.py', 'app', 'file'),
      node('app/model.py::Shape', 'app/model.py', 'class'),
      node('app/model.py::Shape::name', 'app/model.py::Shape', 'field'),
      node('app/model.py::Shape::sides', 'app/model.py::Shape', 'field'),
      node('app/model.py::Shape::area()', 'app/model.py::Shape', 'method'),
      node('app/model.py::Shape::scale()', 'app/model.py::Shape', 'method'),
      node('app/model.py::Shape::describe()', 'app/model.py::Shape', 'method', { lines: 30, columns: 90 }),
      node('lib', '.', 'directory'),
      node('lib/util.py', 'lib', 'file'),
      node('lib/util.py::clamp()', 'lib/util.py', 'function'),
      node('lib/util.py::log()', 'lib/util.py', 'function'),
    ],
    {
      '.': container([wire('app', 'lib')]),
      app: container([wire('app/main.py', 'app/model.py')], [], ['app/main.py', 'app/model.py']),
      'app/main.py': container(
        [wire('app/main.py::run()', 'app/main.py::setup()')],
        [],
        ['app/main.py::run()', 'app/main.py::setup()', 'app/main.py::spare()'],
      ),
      'app/model.py': container([], ['app/model.py::Shape'], ['app/model.py::Shape']),
      'app/model.py::Shape': container(
        [
          wire('app/model.py::Shape::scale()', 'app/model.py::Shape::area()'),
          wire('app/model.py::Shape::describe()', 'app/model.py::Shape::name'),
        ],
        ['app/model.py::Shape::area()', 'app/model.py::Shape::sides'],
        ['app/model.py::Shape::scale()', 'app/model.py::Shape::name'],
      ),
      lib: container([], ['lib/util.py']),
      'lib/util.py': container([], ['lib/util.py::clamp()', 'lib/util.py::log()']),
    },
  )
}
