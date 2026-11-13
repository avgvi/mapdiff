// construct: esm-import-named
import { readFile } from 'node:fs/promises';
// construct: esm-import-default
import Widget from './Widget.jsx';
// construct: esm-import-namespace
import * as math from './math.mjs';

// construct: esm-export-const
export const VERSION = '1.0.0';

// construct: esm-export-function, async-function
export async function readConfig(path) {
  return JSON.parse(await readFile(path, 'utf8'));
}

// construct: esm-re-export
export { add } from './math.mjs';
// construct: esm-re-export-star
export * from './math.mjs';

// construct: esm-export-default-anonymous
export default function (title) {
  return Widget({ title, children: math.add(1, 2) });
}

// construct: dynamic-import
export const lazy = () => import('./math.mjs');

// construct: top-level-await
export const config = await readConfig(new URL('./config.json', import.meta.url)).catch(
  () => ({}),
);
