//! Копирует DLL рантайма VC++ 2015-2022 x64 из System32 в бандл приложения,
//! чтобы движок llama.cpp (MSVC-сборка) запускался «из коробки» без ручной
//! установки Visual C++ Redistributable пользователем.
//!
//! Итоговый вес DLL ~1.2 МБ — ничтожен по сравнению с приложением.
const fs = require('fs');
const path = require('path');

const VC_REDIST_DLLS = [
  'vcruntime140.dll',
  'vcruntime140_1.dll',
  'msvcp140.dll',
  'msvcp140_1.dll',
  'concrt140.dll',
];

function copyVcRedist(scriptDir) {
  const system32 = path.join(process.env.SystemRoot || 'C:\\Windows', 'System32');
  const redistSrc = path.join(scriptDir, 'src-tauri', 'redist');
  fs.mkdirSync(redistSrc, { recursive: true });

  // Для dev-сборки (exe в target/release) кладём DLL в target/release/redist,
  // чтобы ensure_vc_redist нашёл их по пути current_exe().parent()/redist
  // (первый кандидат), не полагаясь на System32.
  const releaseDir = path.join(scriptDir, 'src-tauri', 'target', 'release');
  const targets = [redistSrc];
  const relRedist = path.join(releaseDir, 'redist');
  fs.mkdirSync(relRedist, { recursive: true });
  targets.push(relRedist);
  if (fs.existsSync(releaseDir)) targets.push(releaseDir);

  let copied = 0;
  for (const dll of VC_REDIST_DLLS) {
    const src = path.join(system32, dll);
    if (!fs.existsSync(src)) {
      console.warn(`  ⚠️ ${dll} не найден в ${system32} — рантайм VC++ не попадёт в бандл!`);
      continue;
    }
    for (const t of targets) {
      fs.copyFileSync(src, path.join(t, dll));
    }
    copied++;
  }
  console.log(`  ✓ Скопировано DLL рантайма VC++: ${copied}/${VC_REDIST_DLLS.length}`);
}

module.exports = { copyVcRedist };
