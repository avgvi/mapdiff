// Hand-made fixture. Every JavaScript construct the extractor must map appears
// in this folder, tagged with a `construct:` marker on the line above it. The
// list of required markers is `fixtures/constructs/javascript.txt`.
//
// `lib/` is CommonJS. `esm/` is ES modules and JSX.

// construct: use-strict
'use strict';

// construct: require
const EventEmitter = require('node:events');
// construct: require-destructured
const { createRouter } = require('./router');
// construct: require-member
const debug = require('node:util').debuglog('fixture');

// construct: function
function createApp(options) {
  return new App(withDefaults(options));
}

// construct: class, class-extends
class App extends EventEmitter {
  // construct: class-field
  routes = [];

  // construct: private-field
  #started = false;

  // construct: static-field
  static instances = 0;

  // construct: constructor
  constructor(options = {}) {
    super();
    this.options = options;
    this.router = createRouter();
    App.instances += 1;
  }

  // construct: method
  use(path, handler) {
    this.router.add(path, handler);
    return this;
  }

  // construct: getter
  get started() {
    return this.#started;
  }

  // construct: setter
  set name(value) {
    this.options.name = String(value);
  }

  // construct: static-method
  static create(options) {
    return new App(options);
  }

  // construct: async-method
  async listen(port) {
    this.#started = true;
    this.emit('listening', port);
    return port;
  }

  // construct: private-method
  #reset() {
    this.#started = false;
  }

  stop() {
    this.#reset();
  }
}

// construct: prototype-method
App.prototype.handle = function handle(request) {
  debug('handling %s', request.path);
  return this.router.match(request.path);
};

// construct: function-expression-const
const defaults = function () {
  return { name: 'app' };
};

// construct: arrow-const
const withDefaults = (options) => ({ ...defaults(), ...options });

// construct: dynamic-require
function loadPlugin(name) {
  return require(`./plugins/${name}`);
}

// construct: iife
(function bootstrap() {
  debug('fixture ready');
})();

// construct: module-exports-assignment
module.exports = createApp;
// construct: module-exports-property
module.exports.App = App;
module.exports.loadPlugin = loadPlugin;
