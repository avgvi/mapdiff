// How big a box is, from what it shows. Nothing here renders anything: the
// numbers are the ones the map's style sheet must follow.

import type { Detail, Level, MapNode } from './types.ts'

export const SIZE = {
  /** Width of one character of a title, in a 12px monospace font. */
  char: 7.2,
  /** Height of a title bar. Wires attach to a box at half this height. */
  title: 28,
  /** Room for the kind icon in a title bar. */
  icon: 20,
  /**
   * Space between a container's border and its content, and between two
   * boxes. One number for both: the lane a wire takes along a border and
   * the lane between two boxes are then the same width.
   */
  gap: 16,
  /** Space between two layers of boxes joined by wires. */
  layerGap: 40,
  /** Height of a row. */
  row: 22,
  /** Width of one character of source text, in an 11px monospace font. */
  codeChar: 6.6,
  /** Height of one line of source text. */
  codeLine: 16,
  /** Lines of source text shown before the block is cut. */
  maxCodeLines: 40,
  /** The most lines a block shows once the reader has opened it whole. */
  maxWholeLines: 1500,
  /** Columns shown before a line is cut. */
  maxCodeColumns: 100,
  /** Characters of a title shown before it is cut. */
  maxLabel: 48,
  minWidth: 96,
  /** A small mark: a file inside its directory at the top level. */
  pillWidth: 16,
  pillHeight: 9,
  pillGap: 4,
} as const

const ROW_KINDS = new Set(['field', 'variant', 'import'])
const NEVER_CODE = new Set(['directory', 'file', 'document'])

/** How a node is drawn at a level. */
export function detailOf(node: MapNode, level: Level, hasChildren: boolean): Detail {
  if (hasChildren) return 'container'
  if (node.minor) return 'pill'
  if (ROW_KINDS.has(node.kind)) return 'row'
  if (level === 'code' && !NEVER_CODE.has(node.kind)) return 'code'
  return 'title'
}

/** Kinds whose title starts with the language's own word: `def run`, `impl Ignore`. */
const WORD_KINDS = new Set([
  'module',
  'class',
  'interface',
  'enum',
  'type_alias',
  'impl',
  'function',
  'method',
  'constructor',
  'constant',
  'macro',
])

/** The language's own word for a box, when its title shows one. */
export function wordOf(node: MapNode): string | null {
  // An `impl` already has its word in its name.
  if (!WORD_KINDS.has(node.kind) || node.label.startsWith(`${node.lang_kind} `)) return null
  return node.lang_kind
}

/** What a box's title bar says. */
export function titleOf(node: MapNode): string {
  const word = wordOf(node)
  return word === null ? node.label : `${word} ${node.label}`
}

/** The width a title needs. A container is at least this wide. */
export function titleWidth(label: string): number {
  const characters = Math.min(label.length, SIZE.maxLabel)
  return Math.max(SIZE.minWidth, Math.ceil(characters * SIZE.char + SIZE.icon + SIZE.gap))
}

/** How many lines of text a code box shows, and how many it leaves out. */
export function codeLines(node: MapNode, whole: boolean): { shown: number; hidden: number; foot: boolean } {
  const most = whole ? SIZE.maxWholeLines : SIZE.maxCodeLines
  if (node.lines <= SIZE.maxCodeLines) return { shown: node.lines, hidden: 0, foot: false }
  // A long block ends in a line that opens or closes the rest: one
  // line fewer of text when cut, one line more when whole.
  const shown = whole ? Math.min(node.lines, most) : most - 1
  return { shown, hidden: node.lines - shown, foot: true }
}

/**
 * The size of a box that has no children. `whole` is for a code box the
 * reader has opened past its cut.
 */
export function sizeOf(
  node: MapNode,
  detail: Exclude<Detail, 'container'>,
  whole = false,
): {
  width: number
  height: number
} {
  switch (detail) {
    case 'title':
      return { width: titleWidth(titleOf(node)), height: SIZE.title }
    case 'pill':
      return { width: SIZE.pillWidth, height: SIZE.pillHeight }
    case 'row': {
      const characters = Math.min(node.label.length, SIZE.maxLabel)
      return { width: Math.ceil(characters * SIZE.char + SIZE.gap), height: SIZE.row }
    }
    case 'code': {
      const { shown, foot } = codeLines(node, whole)
      const lines = Math.max(1, shown + (foot ? 1 : 0))
      const columns = Math.min(node.columns, SIZE.maxCodeColumns)
      return {
        width: Math.max(titleWidth(titleOf(node)), Math.ceil(columns * SIZE.codeChar + SIZE.gap)),
        height: SIZE.title + lines * SIZE.codeLine + SIZE.gap / 2,
      }
    }
  }
}

/** How far below its top a wire attaches to a box. */
export function portOffset(row: boolean): number {
  return (row ? SIZE.row : SIZE.title) / 2
}
