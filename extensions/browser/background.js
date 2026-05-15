// Aleph Browser Extension — Background Service Worker
// Captures tab switches, navigation, and bookmarks.
// Sends events to local Aleph daemon at port 2198.

const ALEPH_API = 'http://127.0.0.1:2198/api/ingest/browser';
const SETTINGS_KEY = 'aleph_settings';

let settings = {
  enabled: true,
  captureTabs: true,
  captureBookmarks: true,
  captureHistory: true,
  apiUrl: 'http://127.0.0.1:2198',
};

// Load settings from storage
async function loadSettings() {
  try {
    const result = await chrome.storage.local.get(SETTINGS_KEY);
    if (result[SETTINGS_KEY]) {
      settings = { ...settings, ...result[SETTINGS_KEY] };
      settings.apiUrl = settings.apiUrl || 'http://127.0.0.1:2198';
    }
  } catch (e) {
    console.error('Aleph: Failed to load settings:', e);
  }
}

// Send events to Aleph
async function sendEvents(events) {
  if (!settings.enabled || events.length === 0) return;

  const url = `${settings.apiUrl}/api/ingest/browser`;
  try {
    const resp = await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(events),
    });
    if (!resp.ok) {
      console.warn(`Aleph: ingest returned ${resp.status}`);
    }
  } catch (e) {
    // Aleph not running — quiet fail
  }
}

// Queue events and batch-send
let eventQueue = [];
let flushTimer = null;

function queueEvent(ev) {
  if (!settings.enabled) return;
  eventQueue.push(ev);
  if (!flushTimer) {
    flushTimer = setTimeout(flushEvents, 2000);
  }
}

async function flushEvents() {
  flushTimer = null;
  const batch = eventQueue.slice();
  eventQueue = [];
  if (batch.length > 0) {
    await sendEvents(batch);
  }
}

// ===================================================================
// Tab tracking — capture active tab on switch
// ===================================================================

let lastTabId = null;
let lastTabUrl = null;

chrome.tabs.onActivated.addListener(async (activeInfo) => {
  if (!settings.captureTabs) return;

  // Capture the previously active tab before switching
  if (lastTabId !== null) {
    try {
      const tab = await chrome.tabs.get(lastTabId);
      if (tab.url && tab.url.startsWith('http')) {
        queueEvent({
          url: tab.url,
          title: tab.title || tab.url,
          source_type: 'tab_switch',
          timestamp: Date.now(),
        });
      }
    } catch (e) {
      // Tab may have been closed
    }
  }
  lastTabId = activeInfo.tabId;

  // Capture the newly activated tab
  try {
    const tab = await chrome.tabs.get(activeInfo.tabId);
    if (tab.url && tab.url.startsWith('http')) {
      queueEvent({
        url: tab.url,
        title: tab.title || tab.url,
        source_type: 'tab_switch',
        timestamp: Date.now(),
      });
    }
  } catch (e) {
    // ignore
  }
});

// Capture URL changes within a tab
chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
  if (!settings.captureTabs) return;
  if (changeInfo.url && changeInfo.url.startsWith('http')) {
    queueEvent({
      url: changeInfo.url,
      title: tab.title || changeInfo.url,
      source_type: 'navigation',
      timestamp: Date.now(),
    });
  }
});

// ===================================================================
// Bookmark tracking
// ===================================================================

chrome.bookmarks.onCreated.addListener((id, bookmark) => {
  if (!settings.captureBookmarks) return;
  if (bookmark.url && bookmark.url.startsWith('http')) {
    queueEvent({
      url: bookmark.url,
      title: bookmark.title || bookmark.url,
      source_type: 'bookmark',
      timestamp: Date.now(),
    });
  }
});

// ===================================================================
// History tracking (periodic sync)
// ===================================================================

async function syncHistory() {
  if (!settings.captureHistory) return;

  const oneMinuteAgo = Date.now() - 60000;
  const items = await chrome.history.search({
    text: '',
    startTime: oneMinuteAgo,
    maxResults: 20,
  });

  const events = items
    .filter((item) => item.url && item.url.startsWith('http'))
    .map((item) => ({
      url: item.url,
      title: item.title || item.url,
      source_type: 'history',
      timestamp: item.lastVisitTime || Date.now(),
    }));

  if (events.length > 0) {
    await sendEvents(events);
  }
}

// Periodic history sync every 60 seconds
chrome.alarms.create('historySync', { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === 'historySync') {
    syncHistory();
  }
});

// ===================================================================
// Initialization
// ===================================================================

loadSettings();

// Listen for settings changes
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === 'local' && changes[SETTINGS_KEY]) {
    settings = { ...settings, ...changes[SETTINGS_KEY].newValue };
  }
});
