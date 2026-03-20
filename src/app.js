const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

document.addEventListener("DOMContentLoaded", () => {
    console.log("DictationApp ready");

    // Check for updates silently on launch
    checkForUpdates(false);
});

listen('check-update', () => {
    checkForUpdates(true);
});

async function checkForUpdates(userInitiated) {
    // Anti-loop: skip auto-check if we just updated to this version
    if (!userInitiated) {
        const lastUpdate = localStorage.getItem('lastUpdateVersion');
        const currentVersion = document.getElementById('version').textContent;
        if (lastUpdate === currentVersion) {
            localStorage.removeItem('lastUpdateVersion');
            return;
        }
    }

    try {
        const { check } = await import('@tauri-apps/plugin-updater');
        const { relaunch } = await import('@tauri-apps/plugin-process');

        const update = await check();
        if (update) {
            const shouldUpdate = confirm(
                `Nova versão disponível: v${update.version}\n\nAtualizar agora?`
            );
            if (shouldUpdate) {
                localStorage.setItem('lastUpdateVersion', 'v' + update.version);
                await update.downloadAndInstall();
                await relaunch();
            }
        } else if (userInitiated) {
            alert('Você está usando a versão mais recente.');
        }
    } catch (e) {
        console.error('Update check failed:', e);
        if (userInitiated) {
            alert('Não foi possível verificar atualizações.');
        }
    }
}
