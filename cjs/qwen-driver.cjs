const { createRuntime } = require('./lib/driver-core.cjs');
createRuntime(require('./sites/qwen.cjs')).start();
