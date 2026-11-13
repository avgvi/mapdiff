'use strict';

// construct: module-exports-object
module.exports = {
  name: 'logger',
  install(app) {
    // construct: arrow-callback
    app.on('listening', (port) => console.log(`listening on ${port}`));
  },
};
