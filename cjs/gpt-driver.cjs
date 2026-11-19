const { createRuntime } = require('./lib/driver-core.cjs');
createRuntime(require('./sites/gpt.cjs')).start();
