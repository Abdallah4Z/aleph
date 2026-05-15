const ALEPH_API = 'http://127.0.0.1:2198';

async function checkStatus() {
  const dot = document.getElementById('statusDot');
  const text = document.getElementById('statusText');
  try {
    const resp = await fetch(`${ALEPH_API}/api/capture/status`);
    if (resp.ok) {
      const data = await resp.json();
      dot.className = 'dot on';
      text.textContent = data.enabled ? 'Aleph running — capturing' : 'Aleph running — paused';
    } else {
      dot.className = 'dot off';
      text.textContent = 'Aleph API error';
    }
  } catch {
    dot.className = 'dot off';
    text.textContent = 'Aleph not found';
  }
}

async function loadToggle() {
  const result = await chrome.storage.local.get('aleph_settings');
  const settings = result.aleph_settings || {};
  const btn = document.getElementById('toggleBtn');
  const enabled = settings.enabled !== false;
  btn.textContent = enabled ? '● Capturing' : '○ Paused';
  btn.className = 'btn ' + (enabled ? 'on' : '');
}

document.getElementById('toggleBtn').addEventListener('click', async () => {
  const result = await chrome.storage.local.get('aleph_settings');
  const settings = result.aleph_settings || {};
  settings.enabled = !settings.enabled;
  await chrome.storage.local.set({ aleph_settings: settings });
  loadToggle();
});

checkStatus();
loadToggle();
// Refresh status every 3 seconds
setInterval(checkStatus, 3000);
