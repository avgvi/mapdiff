'use strict';

// construct: constructor-function
function Router() {
  this.routes = new Map();
}

// construct: prototype-method-anonymous
Router.prototype.add = function (path, handler) {
  this.routes.set(helpers.normalise(path), handler);
};

Router.prototype.match = function match(path) {
  return this.routes.get(helpers.normalise(path));
};

// construct: object-literal-method
const helpers = {
  normalise(path) {
    return path.replace(/\/+$/, '') || '/';
  },
  // construct: object-literal-arrow
  isRoot: (path) => path === '/',
};

// construct: generator-function
function* walk(router) {
  yield* router.routes.keys();
}

// construct: exports-property
exports.Router = Router;
// construct: exports-function-expression
exports.createRouter = function createRouter() {
  return new Router();
};
exports.walk = walk;
exports.helpers = helpers;
