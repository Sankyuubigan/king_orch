const fs = require('fs');
const k = fs.readFileSync(process.argv[2], 'utf8').replace(/\r?\n/g, '').trim();
process.stdout.write(k);
