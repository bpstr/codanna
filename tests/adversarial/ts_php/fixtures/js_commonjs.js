const { persist: save } = require('./storage');
exports.run = function run(value) {
    save(value);
};
