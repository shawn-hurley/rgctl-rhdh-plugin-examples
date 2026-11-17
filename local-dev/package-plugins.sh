#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p local-dev/.generated/packages
for plugin in dependency-impact dependency-impact-backend; do
  npm pack "./plugins/$plugin/dist-dynamic" --pack-destination local-dev/.generated/packages
done
node <<'JS'
const fs = require('fs');
const crypto = require('crypto');
const path = require('path');
const root = 'local-dev/.generated/packages';
const packages = ['dependency-impact', 'dependency-impact-backend'].map(plugin => {
  const manifest = JSON.parse(fs.readFileSync(`plugins/${plugin}/dist-dynamic/package.json`, 'utf8'));
  const file = `${manifest.name.replace(/^@/, '').replace(/\//g, '-')}-${manifest.version}.tgz`;
  const integrity = `sha512-${crypto.createHash('sha512').update(fs.readFileSync(path.join(root, file))).digest('base64')}`;
  return { name: manifest.name, version: manifest.version, file, integrity };
});
fs.writeFileSync(path.join(root, 'manifest.json'), `${JSON.stringify({ packages }, null, 2)}\n`);
JS
