//! Hand-labelled references for TypeScript and JavaScript.

mod common;

use common::{assert_edges, graph};

const SHAPES_TS: &str = "\
import { clamp as limit } from \"./util\";
import type { Unit } from \"./util\";
import * as util from \"./util\";
import describe from \"./describe\";
import { Tool } from \"@/tools\";

export interface Shape {
  area(): number;
}

export abstract class Base implements Shape {
  protected unit: Unit = \"m\";
  abstract area(): number;
  describe(): string {
    return describe(this) + this.area();
  }
}

export class Circle extends Base {
  constructor(public radius: number) {
    super();
  }
  area(): number {
    return limit(this.radius, 0, 10) * util.double(this.radius);
  }
  twice(): string {
    return super.describe() + Circle.unit().area();
  }
  static unit(): Circle {
    return new Circle(1);
  }
}

export function make(tool: Tool): Shape {
  const circle = new Circle(1);
  return circle;
}
";

const UTIL_TS: &str = "\
export type Unit = \"m\" | \"ft\";

export function clamp(value: number, low: number, high: number): number {
  return Math.max(low, Math.min(high, value));
}

export const double = (value: number): number => value * 2;
";

const DESCRIBE_TS: &str = "\
import type { Shape } from \"./shapes\";

export default function (shape: Shape): string {
  return String(shape.area());
}
";

const APP_TSX: &str = "\
import { Circle, limitValue } from \"./index\";
import { Badge } from \"./Badge\";

export function App() {
  const circle = new Circle(limitValue(1, 0, 2));
  return <Badge shape={circle} />;
}
";

const BADGE_TSX: &str = "\
import type { Shape } from \"./shapes\";

export function Badge({ shape }: { shape: Shape }) {
  return <b>{shape.area()}</b>;
}
";

#[test]
fn typescript_imports_classes_reexports_and_jsx() {
    let graph = graph(
        "typescript",
        &[
            (
                "tsconfig.json",
                "{\n  // aliases\n  \"compilerOptions\": { \"paths\": { \"@/*\": [\"./src/*\"] } }\n}\n",
            ),
            ("src/shapes.ts", SHAPES_TS),
            ("src/util.ts", UTIL_TS),
            ("src/describe.ts", DESCRIBE_TS),
            ("src/tools.ts", "export class Tool {}\n"),
            (
                "src/index.ts",
                "export * from \"./shapes\";\nexport { clamp as limitValue } from \"./util\";\n",
            ),
            ("src/app.tsx", APP_TSX),
            ("src/Badge.tsx", BADGE_TSX),
        ],
    );
    assert_edges(
        &graph,
        &[
            // Imports: named with an alias, type-only, namespace, default,
            // and through a tsconfig path alias.
            "src/shapes.ts::{import.1} -> src/util.ts::clamp() [import]",
            "src/shapes.ts::{import.2} -> src/util.ts::Unit [import]",
            "src/shapes.ts::{import.3} -> src/util.ts [import]",
            "src/shapes.ts::{import.4} -> src/describe.ts::{default} [import]",
            "src/shapes.ts::{import.5} -> src/tools.ts::Tool [import]",
            "src/describe.ts::{import.1} -> src/shapes.ts::Shape [import]",
            "src/Badge.tsx::{import.1} -> src/shapes.ts::Shape [import]",
            // Re-exports: `export *` and a renamed `export { } from`.
            "src/index.ts::{import.1} -> src/shapes.ts [import]",
            "src/index.ts::{import.2} -> src/util.ts::clamp() [import]",
            "src/app.tsx::{import.1} -> src/shapes.ts::Circle [import]",
            "src/app.tsx::{import.1} -> src/util.ts::clamp() [import]",
            "src/app.tsx::{import.2} -> src/Badge.tsx::Badge() [import]",
            // Class relations.
            "src/shapes.ts::Base -> src/shapes.ts::Shape [implement]",
            "src/shapes.ts::Circle -> src/shapes.ts::Base [inherit]",
            // Types in annotations.
            "src/shapes.ts::Base::unit -> src/util.ts::Unit [type]",
            "src/shapes.ts::make() -> src/tools.ts::Tool [type]",
            "src/shapes.ts::make() -> src/shapes.ts::Shape [type]",
            "src/describe.ts::{default} -> src/shapes.ts::Shape [type]",
            "src/Badge.tsx::Badge() -> src/shapes.ts::Shape [type]",
            // Calls: a default import, `this.`, an alias, a namespace
            // member, `super.`, a static method, `new`.
            "src/shapes.ts::Base::describe() -> src/describe.ts::{default} [call]",
            "src/shapes.ts::Base::describe() -> src/shapes.ts::Base::area() [call]",
            "src/shapes.ts::Circle::area() -> src/util.ts::clamp() [call]",
            "src/shapes.ts::Circle::area() -> src/util.ts::double() [call]",
            "src/shapes.ts::Circle::twice() -> src/shapes.ts::Base::describe() [call]",
            "src/shapes.ts::Circle::twice() -> src/shapes.ts::Circle::unit() [call]",
            "src/shapes.ts::make() -> src/shapes.ts::Circle [call]",
            // A field declared as a constructor parameter, through `this`.
            "src/shapes.ts::Circle::area() -> src/shapes.ts::Circle::constructor()::radius [access]",
            // Through a barrel file, and a component used as a JSX tag.
            "src/app.tsx::App() -> src/shapes.ts::Circle [call]",
            "src/app.tsx::App() -> src/util.ts::clamp() [call]",
            "src/app.tsx::App() -> src/Badge.tsx::Badge() [call]",
        ],
    );
    // Not found, and not expected of the fast tier: `shape.area()` in
    // `describe.ts` and `Badge.tsx` (a method on a typed parameter), and
    // `.area()` on the value `Circle.unit()` returns.
}

const APP_JS: &str = "\
'use strict';

const EventEmitter = require('node:events');
const { createRouter, Router } = require('./router');
const helpers = require('./helpers');

class App extends EventEmitter {
  constructor() {
    super();
    this.router = createRouter();
  }

  start() {
    helpers.log('start');
    return this.handle();
  }

  handle() {
    return new Router();
  }
}

function createApp() {
  return new App();
}

module.exports = createApp;
module.exports.App = App;
";

const ROUTER_JS: &str = "\
function Router() {
  this.routes = [];
}

Router.prototype.add = function (path) {
  this.routes.push(normalise(path));
};

function normalise(path) {
  return path;
}

exports.Router = Router;
exports.createRouter = function createRouter() {
  return new Router();
};
";

const MAIN_MJS: &str = "\
import createApp, { App } from './lib/app.js';

export function run() {
  const app = createApp();
  return new App();
}
";

#[test]
fn javascript_require_exports_and_prototypes() {
    let graph = graph(
        "javascript",
        &[
            ("lib/app.js", APP_JS),
            ("lib/router.js", ROUTER_JS),
            (
                "lib/helpers.js",
                "module.exports = {\n  log(message) {\n    console.log(message);\n  },\n};\n",
            ),
            ("main.mjs", MAIN_MJS),
        ],
    );
    assert_edges(
        &graph,
        &[
            // `require`: destructured names and a whole module.
            "lib/app.js::{import.2} -> lib/router.js::createRouter() [import]",
            "lib/app.js::{import.2} -> lib/router.js::Router [import]",
            "lib/app.js::{import.3} -> lib/helpers.js [import]",
            // An ES module importing a CommonJS one: the function assigned
            // to `module.exports` is its default export.
            "main.mjs::{import.1} -> lib/app.js::createApp() [import]",
            "main.mjs::{import.1} -> lib/app.js::App [import]",
            "main.mjs::run() -> lib/app.js::createApp() [call]",
            "main.mjs::run() -> lib/app.js::App [call]",
            // Calls: a destructured import, a method of an exported object,
            // `this.`, a constructor function, `new`.
            "lib/app.js::App::constructor() -> lib/router.js::createRouter() [call]",
            "lib/app.js::App::start() -> lib/helpers.js::{exports}::log() [call]",
            "lib/app.js::App::start() -> lib/app.js::App::handle() [call]",
            "lib/app.js::App::handle() -> lib/router.js::Router [call]",
            "lib/app.js::createApp() -> lib/app.js::App [call]",
            "lib/router.js::createRouter() -> lib/router.js::Router [call]",
            "lib/router.js::Router.prototype.add() -> lib/router.js::normalise() [call]",
            // The statements that export names refer to them.
            "lib/app.js::{block.1} -> lib/app.js::createApp() [access]",
            "lib/app.js::{block.1} -> lib/app.js::App [access]",
            "lib/router.js::{block.1} -> lib/router.js::Router [access]",
            // A method attached to a prototype names the function it extends.
            "lib/router.js::Router.prototype.add() -> lib/router.js::Router [access]",
        ],
    );
    // Not found, and not expected: `App extends EventEmitter` (a Node
    // built-in, outside the repository), and `this.routes` inside the
    // prototype method (the fast tier does not know what `this` is there).
}
