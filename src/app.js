const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// DOM elements
const statusIcon = document.getElementById('status-icon');
const statusText = document.getElementById('status-text');
const transcription = document.getElementById('transcription');
const errorMessage = document.getElementById('error-message');
const historyList = document.getElementById('history-list');
const downloadOverlay = document.getElementById('download-overlay');
const batchStatus = document.getElementById('batch-status');
const batchStatusText = document.getElementById('batch-status-text');

let currentState = 'idle';

// Initialize
document.addEventListener('DOMContentLoaded', async () => {
    await checkAndDownloadModels();
    await loadModels();
    await loadHistory();
    setupModelToggles();
    setupEventListeners();
});

// Model download on first launch
async function checkAndDownloadModels() {
    const status = await invoke('check_models');
    const needsDownload = !status.small; // Small is required

    if (needsDownload) {
        downloadOverlay.classList.remove('hidden');

        // Download Small first (required)
        if (!status.small) {
            await invoke('download_model_cmd', { name: 'small' });
        }
        // Then Turbo (optional, background)
        if (!status.turbo) {
            invoke('download_model_cmd', { name: 'turbo' }); // don't await
        }

        downloadOverlay.classList.add('hidden');
    }
}

async function loadModels() {
    await invoke('load_models');
}

async function loadHistory() {
    const history = await invoke('get_history');
    renderHistory(history);
}

function renderHistory(items) {
    historyList.innerHTML = '';
    items.forEach(text => {
        const li = document.createElement('li');
        li.textContent = text;
        li.title = text;
        li.addEventListener('click', () => {
            invoke('reuse_history_item', { text });
        });
        historyList.appendChild(li);
    });
}

// Model toggle buttons
function setupModelToggles() {
    document.querySelectorAll('#live-model-toggle .toggle-btn').forEach(btn => {
        btn.addEventListener('click', () => {
            document.querySelectorAll('#live-model-toggle .toggle-btn').forEach(b => b.classList.remove('active'));
            btn.classList.add('active');
            invoke('set_live_model', { name: btn.dataset.model });
        });
    });

    document.querySelectorAll('#batch-model-toggle .toggle-btn').forEach(btn => {
        btn.addEventListener('click', () => {
            document.querySelectorAll('#batch-model-toggle .toggle-btn').forEach(b => b.classList.remove('active'));
            btn.classList.add('active');
            invoke('set_batch_model', { name: btn.dataset.model });
        });
    });
}

// Tauri event listeners
function setupEventListeners() {
    listen('state-change', (event) => {
        currentState = event.payload;
        updateStatusUI();
    });

    listen('streaming-update', (event) => {
        transcription.value = event.payload;
        transcription.scrollTop = transcription.scrollHeight;
    });

    listen('transcription-complete', (event) => {
        transcription.value = event.payload;
    });

    listen('transcription-error', (event) => {
        errorMessage.textContent = event.payload;
        errorMessage.classList.remove('hidden');
        setTimeout(() => errorMessage.classList.add('hidden'), 5000);
    });

    listen('download-progress', (event) => {
        const { name, downloaded, total } = event.payload;
        const pct = total > 0 ? Math.round((downloaded / total) * 100) : 0;
        const fill = document.getElementById(`progress-${name}`);
        const text = document.getElementById(`progress-${name}-text`);
        if (fill) fill.style.width = `${pct}%`;
        if (text) text.textContent = `${pct}%`;
    });

    listen('batch-progress', (event) => {
        batchStatus.classList.remove('hidden');
        batchStatusText.textContent = event.payload;
    });

    listen('batch-complete', (event) => {
        batchStatus.classList.add('hidden');
    });

    listen('history-updated', async () => {
        await loadHistory();
    });
}

function updateStatusUI() {
    switch (currentState) {
        case 'idle':
            statusIcon.style.color = '#4ecca3';
            statusText.textContent = 'Ready';
            break;
        case 'recording':
            statusIcon.style.color = '#ff6b6b';
            statusText.textContent = 'Recording...';
            break;
        case 'transcribing':
            statusIcon.style.color = '#ffd93d';
            statusText.textContent = 'Transcribing...';
            break;
    }
}
