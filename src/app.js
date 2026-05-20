const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const statusIcon = document.getElementById('status-icon');
const statusSpinner = document.getElementById('status-spinner');
const statusCheck = document.getElementById('status-check');
const statusText = document.getElementById('status-text');
const statusTimer = document.getElementById('status-timer');
const transcription = document.getElementById('transcription');
const errorMessage = document.getElementById('error-message');
const historyList = document.getElementById('history-list');
const downloadOverlay = document.getElementById('download-overlay');
const batchStatus = document.getElementById('batch-status');
const batchStatusText = document.getElementById('batch-status-text');
const batchProgress = document.getElementById('batch-progress');
const footerHotkey = document.getElementById('footer-hotkey');
const translationModeSelect = document.getElementById('translation-mode');

let currentState = 'idle';
let timerStart = null;
let timerInterval = null;

document.addEventListener('DOMContentLoaded', async () => {
    await checkAndDownloadModels();
    await loadModels();
    await loadSettings();
    await loadHistory();

    setupModelToggles();
    setupActivationToggle();
    setupTranslationMode();
    setupEventListeners();
});

async function checkAndDownloadModels() {
    try {
        const status = await invoke('check_models');
        const needsDownload = !status.small;

        if (!needsDownload) {
            return;
        }

        downloadOverlay.classList.remove('hidden');

        if (!status.small) {
            await invoke('download_model_cmd', { name: 'small' });
        }
        if (!status.turbo) {
            invoke('download_model_cmd', { name: 'turbo' });
        }

        // The backend command spawns the download threads and returns.
        // Keep overlay briefly to avoid flicker on fast paths.
        setTimeout(() => downloadOverlay.classList.add('hidden'), 1200);
    } catch (err) {
        showError(`Erro ao validar modelos: ${err}`);
        downloadOverlay.classList.add('hidden');
    }
}

async function loadModels() {
    try {
        await invoke('load_models');
    } catch (err) {
        showError(`Erro ao carregar modelos: ${err}`);
    }
}

async function loadSettings() {
    try {
        const settings = await invoke('get_settings');

        setToggleActive('#live-model-toggle .toggle-btn', settings.live_model, 'model');
        setToggleActive('#batch-model-toggle .toggle-btn', settings.batch_model, 'model');
        setToggleActive('#activation-key-toggle .toggle-btn', settings.activation_key, 'key');

        translationModeSelect.value = settings.translation_mode || 'off';
        updateFooterHotkey(settings.activation_key);
    } catch (err) {
        showError(`Erro ao carregar configurações: ${err}`);
    }
}

async function loadHistory() {
    try {
        const history = await invoke('get_history');
        renderHistory(history);
    } catch (err) {
        showError(`Erro ao carregar histórico: ${err}`);
    }
}

function renderHistory(items) {
    historyList.innerHTML = '';
    items.forEach((text) => {
        const li = document.createElement('li');
        li.textContent = text;
        li.title = text;
        li.addEventListener('click', () => {
            invoke('reuse_history_item', { text });
        });
        historyList.appendChild(li);
    });
}

function setupModelToggles() {
    document.querySelectorAll('#live-model-toggle .toggle-btn').forEach((btn) => {
        btn.addEventListener('click', () => {
            document
                .querySelectorAll('#live-model-toggle .toggle-btn')
                .forEach((b) => b.classList.remove('active'));
            btn.classList.add('active');
            invoke('set_live_model', { name: btn.dataset.model });
        });
    });

    document.querySelectorAll('#batch-model-toggle .toggle-btn').forEach((btn) => {
        btn.addEventListener('click', () => {
            document
                .querySelectorAll('#batch-model-toggle .toggle-btn')
                .forEach((b) => b.classList.remove('active'));
            btn.classList.add('active');
            invoke('set_batch_model', { name: btn.dataset.model });
        });
    });
}

function setupActivationToggle() {
    document.querySelectorAll('#activation-key-toggle .toggle-btn').forEach((btn) => {
        btn.addEventListener('click', () => {
            document
                .querySelectorAll('#activation-key-toggle .toggle-btn')
                .forEach((b) => b.classList.remove('active'));
            btn.classList.add('active');
            const key = btn.dataset.key;
            invoke('set_activation_key', { key });
            updateFooterHotkey(key);
        });
    });
}

function setupTranslationMode() {
    translationModeSelect.addEventListener('change', () => {
        invoke('set_translation_mode', { mode: translationModeSelect.value });
    });
}

function setToggleActive(selector, value, datasetField) {
    document.querySelectorAll(selector).forEach((btn) => {
        if (btn.dataset[datasetField] === value) {
            btn.classList.add('active');
        } else {
            btn.classList.remove('active');
        }
    });
}

function updateFooterHotkey(key) {
    if (key === 'win') {
        footerHotkey.textContent = 'Win Win para ditar';
        transcription.placeholder = 'Win Win para ditar...';
        return;
    }

    footerHotkey.textContent = 'Ctrl Ctrl para ditar';
    transcription.placeholder = 'Ctrl Ctrl para ditar...';
}

function showError(message) {
    errorMessage.textContent = message;
    errorMessage.classList.remove('hidden');
    setTimeout(() => errorMessage.classList.add('hidden'), 5000);
}

function setupEventListeners() {
    listen('state-change', (event) => {
        console.log('[state-change]', event.payload);
        currentState = event.payload?.state || 'idle';
        updateStatusUI();
    });

    listen('streaming-update', (event) => {
        const text = event.payload?.text || '';
        transcription.value = text;
        transcription.scrollTop = transcription.scrollHeight;
    });

    listen('transcription-complete', (event) => {
        const text = event.payload?.final_text || '';
        transcription.value = text;
        currentState = 'idle';
        showCompletion();
    });

    listen('transcription-error', (event) => {
        showError(event.payload?.message || 'Erro desconhecido');
        currentState = 'idle';
        updateStatusUI();
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
        const payload = event.payload || {};
        batchStatus.classList.remove('hidden');

        const pct = payload.total > 0 ? Math.round((payload.index / payload.total) * 100) : 0;
        batchProgress.style.width = `${pct}%`;

        if (payload.error) {
            batchStatusText.textContent = `Erro (${payload.index}/${payload.total}): ${payload.file} - ${payload.error}`;
        } else {
            const excerpt = (payload.text || '').slice(0, 90);
            batchStatusText.textContent = `${payload.index}/${payload.total} ${payload.file}${excerpt ? `: ${excerpt}` : ''}`;
        }

        if (payload.done) {
            setTimeout(() => {
                batchStatus.classList.add('hidden');
                batchProgress.style.width = '0%';
                batchStatusText.textContent = '';
            }, 1500);
        }
    });

    listen('history-updated', async () => {
        await loadHistory();
    });

    listen('open-file-dialog', async () => {
        const selectedPath = await invoke('pick_file_cmd');
        if (!selectedPath) {
            return;
        }
        await invoke('transcribe_file_cmd', { path: selectedPath });
    });

    listen('open-folder-dialog', async () => {
        const selectedPath = await invoke('pick_folder_cmd');
        if (!selectedPath) {
            return;
        }
        await invoke('transcribe_folder_cmd', { path: selectedPath });
    });

    listen('check-update', () => {
        showError('Check for Updates ainda não implementado nesta build.');
    });
}

function formatTime(seconds) {
    const mins = Math.floor(seconds / 60);
    const secs = Math.floor(seconds % 60);
    return `${mins}:${secs.toString().padStart(2, '0')}`;
}

function startTimer() {
    timerStart = Date.now();
    statusTimer.classList.remove('hidden');
    statusTimer.textContent = '0:00';

    if (timerInterval) clearInterval(timerInterval);
    timerInterval = setInterval(() => {
        const elapsed = (Date.now() - timerStart) / 1000;
        statusTimer.textContent = formatTime(elapsed);
    }, 100);
}

function stopTimer() {
    if (timerInterval) {
        clearInterval(timerInterval);
        timerInterval = null;
    }
}

function showCompletion() {
    stopTimer();

    statusIcon.classList.add('hidden');
    statusIcon.classList.remove('recording');
    statusSpinner.classList.add('hidden');
    statusCheck.classList.remove('hidden');
    statusTimer.classList.add('hidden');
    statusText.textContent = 'Done';

    setTimeout(() => {
        statusCheck.classList.add('hidden');
        statusIcon.classList.remove('hidden');
        statusText.textContent = 'Ready';
    }, 1000);
}

function updateStatusUI() {
    console.log('[updateStatusUI] state:', currentState);
    switch (currentState) {
        case 'idle':
            stopTimer();
            statusIcon.classList.remove('hidden', 'recording');
            statusSpinner.classList.add('hidden');
            statusCheck.classList.add('hidden');
            statusTimer.classList.add('hidden');
            statusTimer.classList.remove('recording', 'transcribing');
            statusText.textContent = 'Ready';
            break;
        case 'recording':
            statusIcon.classList.remove('hidden');
            statusIcon.classList.add('recording');
            statusSpinner.classList.add('hidden');
            statusCheck.classList.add('hidden');
            statusTimer.classList.remove('transcribing');
            statusTimer.classList.add('recording');
            statusText.textContent = 'Recording...';
            startTimer();
            break;
        case 'transcribing':
            statusIcon.classList.add('hidden');
            statusIcon.classList.remove('recording');
            statusSpinner.classList.remove('hidden');
            statusCheck.classList.add('hidden');
            statusTimer.classList.remove('recording');
            statusTimer.classList.add('transcribing');
            statusText.textContent = 'Transcribing...';
            break;
        default:
            stopTimer();
            statusIcon.classList.remove('hidden', 'recording');
            statusSpinner.classList.add('hidden');
            statusCheck.classList.add('hidden');
            statusTimer.classList.add('hidden');
            statusText.textContent = 'Ready';
            break;
    }
}
