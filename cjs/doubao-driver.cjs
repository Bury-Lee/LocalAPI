const { createRuntime } = require('./lib/driver-core.cjs');
createRuntime(require('./sites/doubao.cjs')).start();
