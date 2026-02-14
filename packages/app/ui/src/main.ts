import "./style.css";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";

const IDLE_THRESHOLD_MS = 5 * 60 * 1000; // 5 minutes

// Cleanup tracking for memory leak prevention
let fetchDataIntervalId: number | null = null;
let checkIdleIntervalId: number | null = null;
let rafId: number | null = null;
let isAppRunning = true;

let idleDialogOpen = false;
let currentStatus: Status | null = null;
let knownProjectIds: Set<string> = new Set();
let localManualMode: Map<string, { active: boolean; startTime: number }> = new Map();
let frozenTimes: Map<string, { weekTime: number; todayTime: number }> = new Map(); // Frozen display times after stopping
let lastRenderedTimes: Map<string, { weekTime: number; todayTime: number }> = new Map(); // What renderTimers last calculated
let lastFetchTime = Date.now();
let lastRenderSecond = -1;
let lastProjectStates: Map<string, string> = new Map(); // For shallow comparison

interface Project {
  id: string;
  name: string;
  path: string;
  color: string;
  hourlyRate: number | null;
  isTracking: boolean;
  manualMode: boolean;
  elapsedTime: number;
  todayTime: number;
  weekTime: number;
  totalTime: number;
  claudeState: "active" | "stopped";
  claudeSessionCount: number;
}

interface BusinessInfo {
  name: string;
  email: string | null;
  taxRate: number;
}

interface Status {
  projects: Project[];
  todayTotal: number;
  claudeTotal: number;
  systemIdleTime: number;
}

interface TimeEntry {
  id: string;
  projectId: string;
  startTime: number;
  endTime: number;
}

function formatDuration(ms: number, showSeconds = true): string {
  const seconds = Math.floor(ms / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);

  if (showSeconds) {
    if (hours > 0) {
      return `${hours}h ${minutes % 60}m ${seconds % 60}s`;
    } else if (minutes > 0) {
      return `${minutes}m ${seconds % 60}s`;
    } else {
      return `${seconds}s`;
    }
  } else {
    if (hours > 0) {
      return `${hours}h ${minutes % 60}m`;
    } else {
      return `${minutes}m`;
    }
  }
}

function formatEarnings(ms: number, hourlyRate: number | null): string {
  if (!hourlyRate) return "";
  const hours = ms / (1000 * 60 * 60);
  const amount = hours * hourlyRate;
  return `$${amount.toFixed(2)}`;
}

// Tauri invoke wrappers
async function fetchStatus(): Promise<Status> {
  return invoke("get_status");
}

async function startTracking(projectId: string): Promise<void> {
  await invoke("start_tracking", { projectId, manualMode: true });
}

async function stopTracking(projectId: string, endTime?: number): Promise<void> {
  await invoke("stop_tracking", { projectId, endTime: endTime ?? null });
}

async function deleteProject(projectId: string): Promise<void> {
  await invoke("delete_project", { projectId });
}

async function updateProjectRate(projectId: string, hourlyRate: number | null): Promise<void> {
  await invoke("update_project_rate", { projectId, hourlyRate });
}

async function updateProjectName(projectId: string, name: string): Promise<void> {
  await invoke("update_project_name", { projectId, name });
}

async function addProject(name: string, path: string): Promise<void> {
  await invoke("create_project", { name, path });
}

async function getBusinessInfo(): Promise<BusinessInfo> {
  return invoke("get_business_info");
}

async function saveBusinessInfo(info: BusinessInfo): Promise<void> {
  await invoke("save_business_info", {
    name: info.name,
    email: info.email,
    taxRate: info.taxRate,
  });
}

async function generateInvoice(projectId: string, startDate: number, endDate: number, extraHours: number): Promise<string> {
  return invoke("generate_invoice", { projectId, startDate, endDate, extraHours });
}

async function fetchEntries(projectId: string, dayStart?: number): Promise<TimeEntry[]> {
  return invoke("get_entries", { projectId, dayStart: dayStart ?? null });
}

async function deleteEntry(entryId: string): Promise<void> {
  await invoke("delete_entry", { entryId });
}

async function updateEntry(entryId: string, startTime: number, endTime: number): Promise<void> {
  await invoke("update_entry", { entryId, startTime, endTime });
}

async function addTimeEntry(projectId: string, startTime: number, endTime: number): Promise<TimeEntry> {
  return invoke("add_time_entry", { projectId, startTime, endTime });
}

interface HooksStatus {
  scriptInstalled: boolean;
  settingsConfigured: boolean;
  fullyInstalled: boolean;
}

async function checkHooksInstalled(): Promise<HooksStatus> {
  return invoke("check_hooks_installed");
}

async function installHooks(): Promise<HooksStatus> {
  return invoke("install_hooks");
}

function generateUUID(): string {
  return crypto.randomUUID();
}

// Create a project card element
function createProjectCard(p: Project): HTMLElement {
  const card = document.createElement("div");
  card.className = `project ${p.isTracking ? "tracking" : ""}`;
  card.style.borderLeftColor = p.color;
  card.id = `project-${p.id}`;
  card.dataset.projectId = p.id;

  card.innerHTML = `
    <div class="project-header">
      <h3>${p.name}${p.hourlyRate ? `<span class="project-rate">$${p.hourlyRate}/hr</span>` : ""}</h3>
      <div class="project-header-right">
        <div class="status-icons" id="icons-${p.id}"></div>
        <div class="menu-container">
          <button class="btn-icon btn-menu">⋯</button>
          <div class="menu-dropdown">
            <button class="menu-item btn-activity">View Activity</button>
            <button class="menu-item btn-rename">Rename</button>
            <button class="menu-item btn-rate">Set Rate</button>
            <button class="menu-item btn-invoice">Generate Invoice</button>
            <button class="menu-item btn-delete">Delete</button>
          </div>
        </div>
      </div>
    </div>
    <div class="project-path">${p.path}</div>
    <div class="project-bottom">
      <div class="project-stats">
        <span class="stat">
          <span class="stat-label">Week</span>
          <span class="stat-value" id="week-${p.id}">${formatDuration(p.weekTime)}</span>
          <span class="stat-earnings" id="week-earnings-${p.id}">${formatEarnings(p.weekTime, p.hourlyRate)}</span>
        </span>
        <span class="stat">
          <span class="stat-label">Today</span>
          <span class="stat-value" id="today-${p.id}">${formatDuration(p.todayTime)}</span>
          <span class="stat-earnings" id="today-earnings-${p.id}">${formatEarnings(p.todayTime, p.hourlyRate)}</span>
        </span>
      </div>
      <button class="btn-play ${p.manualMode ? "btn-stop" : "btn-start"}" id="playbtn-${p.id}">
        ${p.manualMode ? "◼" : "▶"}
      </button>
    </div>
  `;

  // Event: double-click to toggle
  card.addEventListener("dblclick", (e) => {
    const target = e.target as HTMLElement;
    if (target.closest("button") || target.closest(".menu-container")) return;

    const local = localManualMode.get(p.id);
    const isActive = local?.active ?? false;

    if (isActive) {
      // Stop - update local state immediately for instant UI
      localManualMode.set(p.id, { active: false, startTime: 0 });
      stopTracking(p.id).then(() => fetchData()); // Sync to backend, then refresh totals
    } else {
      // Start - update local state immediately for instant UI
      localManualMode.set(p.id, { active: true, startTime: Date.now() });
      startTracking(p.id); // Background sync to backend
    }

    // Update UI immediately from local state
    const current = currentStatus?.projects.find(proj => proj.id === p.id);
    if (current) {
      current.manualMode = !isActive;
      current.isTracking = !isActive || current.claudeState === "active";
      updateProjectCard(current);
    }
  });

  // Event: menu toggle
  const menuBtn = card.querySelector(".btn-menu")!;
  const dropdown = card.querySelector(".menu-dropdown")!;
  menuBtn.addEventListener("click", (e) => {
    e.stopPropagation();
    // Close all other project menus
    document.querySelectorAll(".menu-dropdown.open").forEach(d => d.classList.remove("open"));
    // Close header menu when opening project menu
    const headerMenu = document.getElementById("header-menu");
    if (headerMenu) headerMenu.classList.remove("open");
    dropdown.classList.toggle("open");
  });

  // Event: view activity
  card.querySelector(".btn-activity")!.addEventListener("click", async (e) => {
    e.stopPropagation();
    dropdown.classList.remove("open");
    showActivityModal(p);
  });

  // Event: rename
  const renameBtn = card.querySelector(".btn-rename");
  if (renameBtn) {
    renameBtn.addEventListener("click", async (e) => {
      e.stopPropagation();
      dropdown.classList.remove("open");
      showRenameDialog(p);
    });
  }

  // Event: delete
  const deleteBtn = card.querySelector(".btn-delete");
  if (deleteBtn) {
    deleteBtn.addEventListener("click", async (e) => {
      e.stopPropagation();
      dropdown.classList.remove("open");
      showConfirmDialog(`Delete "${p.name}"?`, async () => {
        try {
          await deleteProject(p.id);
          await rebuildProjects();
        } catch (err) {
          console.error("Failed to delete project:", err);
          alert(`Failed to delete project: ${err}`);
        }
      });
    });
  }

  // Event: set rate
  const rateBtn = card.querySelector(".btn-rate");
  if (rateBtn) {
    rateBtn.addEventListener("click", async (e) => {
      e.stopPropagation();
      dropdown.classList.remove("open");
      showRateDialog(p);
    });
  }

  // Event: generate invoice
  const invoiceBtn = card.querySelector(".btn-invoice");
  if (invoiceBtn) {
    invoiceBtn.addEventListener("click", async (e) => {
      e.stopPropagation();
      dropdown.classList.remove("open");
      showInvoiceDialog(p);
    });
  }

  // Event: play/stop button - instant local state update
  const playBtn = card.querySelector(".btn-play")! as HTMLButtonElement;
  playBtn.addEventListener("click", (e) => {
    e.stopPropagation();

    const local = localManualMode.get(p.id);
    const isActive = local?.active ?? false;

    if (isActive) {
      // Freeze the exact times that renderTimers last calculated and displayed
      const lastRendered = lastRenderedTimes.get(p.id);
      if (lastRendered) {
        frozenTimes.set(p.id, {
          weekTime: lastRendered.weekTime,
          todayTime: lastRendered.todayTime
        });
      }

      localManualMode.set(p.id, { active: false, startTime: 0 });
      stopTracking(p.id).then(() => fetchData()); // Sync to backend, then refresh totals
    } else {
      // Start - update local state immediately for instant UI
      localManualMode.set(p.id, { active: true, startTime: Date.now() });
      startTracking(p.id); // Background sync to backend
    }

    // Update UI immediately from local state
    const current = currentStatus?.projects.find(proj => proj.id === p.id);
    if (current) {
      current.manualMode = !isActive;
      current.isTracking = !isActive || current.claudeState === "active";
      updateProjectCard(current);
    }
  });

  updateProjectCard(p);
  return card;
}

// Update an existing project card with new data (with shallow comparison)
function updateProjectCard(p: Project): void {
  // Create state key for comparison (only properties that affect rendering)
  const stateKey = `${p.isTracking}|${p.manualMode}|${p.claudeState}`;
  const lastState = lastProjectStates.get(p.id);

  // Skip if nothing changed
  if (lastState === stateKey) return;
  lastProjectStates.set(p.id, stateKey);

  const card = document.getElementById(`project-${p.id}`);
  if (!card) return;

  // Update tracking class
  card.className = `project ${p.isTracking ? "tracking" : ""}`;

  // Note: times are updated by renderTimers() for smooth display

  // Update icons
  const iconsEl = document.getElementById(`icons-${p.id}`);
  if (iconsEl) {
    let iconsHtml = "";
    if (p.claudeState === "active") {
      iconsHtml += `<span class="status-icon claude-active" title="Claude active">✦</span>`;
    }
    if (p.isTracking && p.manualMode) {
      iconsHtml += `<span class="status-icon manual-mode" title="Manual mode">✋</span>`;
    }
    iconsEl.innerHTML = iconsHtml;
  }

  // Update play button
  const playBtn = document.getElementById(`playbtn-${p.id}`);
  if (playBtn) {
    playBtn.className = `btn-play ${p.manualMode ? "btn-stop" : "btn-start"}`;
    playBtn.textContent = p.manualMode ? "◼" : "▶";
  }
}

// Build the initial shell (header, form, etc)
function buildShell(): void {
  const app = document.getElementById("app")!;
  app.innerHTML = `
    <header>
      <div class="header-stats">
        <span class="stat">
          <span class="stat-label">Week</span>
          <span class="stat-value" id="header-week">0s</span>
          <span class="stat-earnings" id="header-week-earnings"></span>
        </span>
        <span class="stat">
          <span class="stat-label">Today</span>
          <span class="stat-value" id="header-today">0s</span>
          <span class="stat-earnings" id="header-today-earnings"></span>
        </span>
      </div>
      <div class="header-right">
        <div class="menu-container">
          <button class="btn-icon btn-header-menu" id="header-menu-btn">⋯</button>
          <div class="menu-dropdown" id="header-menu">
            <button class="menu-item" id="btn-invoices">View Invoices</button>
            <button class="menu-item" id="btn-open-data">Open Data Folder</button>
            <button class="menu-item" id="btn-settings">Settings</button>
          </div>
        </div>
      </div>
    </header>

    <main>
      <div class="projects" id="projects-container"></div>

      <div class="add-project">
        <h3>Add Project</h3>
        <form id="add-form">
          <input type="text" id="project-name" placeholder="Project name" required />
          <div class="path-input-row">
            <input type="text" id="project-path" placeholder="Path" required />
            <button type="button" class="btn btn-browse" id="btn-browse">📁</button>
          </div>
          <button type="submit" class="btn btn-add">Add</button>
        </form>
      </div>
    </main>
  `;

  // Header menu toggle
  document.getElementById("header-menu-btn")!.addEventListener("click", (e) => {
    e.stopPropagation();
    // Close all project menus when opening header menu
    document.querySelectorAll(".menu-dropdown.open").forEach(d => d.classList.remove("open"));
    document.getElementById("header-menu")!.classList.toggle("open");
  });

  // Invoices button - opens invoices folder
  document.getElementById("btn-invoices")!.addEventListener("click", async (e) => {
    e.stopPropagation();
    document.getElementById("header-menu")!.classList.remove("open");
    await invoke("open_invoices_folder");
  });

  // Settings button
  document.getElementById("btn-settings")!.addEventListener("click", (e) => {
    e.stopPropagation();
    document.getElementById("header-menu")!.classList.remove("open");
    showSettingsModal();
  });

  // Open data folder button
  document.getElementById("btn-open-data")!.addEventListener("click", async (e) => {
    e.stopPropagation();
    document.getElementById("header-menu")!.classList.remove("open");
    await invoke("open_data_folder");
  });

  // Close menus when clicking outside
  document.addEventListener("click", () => {
    document.querySelectorAll(".menu-dropdown.open").forEach(d => d.classList.remove("open"));
    const headerMenu = document.getElementById("header-menu");
    if (headerMenu) headerMenu.classList.remove("open");
  });

  // Browse button
  document.getElementById("btn-browse")!.addEventListener("click", async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "Select Project Folder"
    });
    if (selected) {
      (document.getElementById("project-path") as HTMLInputElement).value = selected as string;
    }
  });

  // Form submit
  document.getElementById("add-form")!.addEventListener("submit", async (e) => {
    e.preventDefault();
    const nameInput = document.getElementById("project-name") as HTMLInputElement;
    const pathInput = document.getElementById("project-path") as HTMLInputElement;
    await addProject(nameInput.value, pathInput.value);
    nameInput.value = "";
    pathInput.value = "";
    rebuildProjects();
  });
}

// Rebuild all project cards (only when projects added/removed)
async function rebuildProjects(): Promise<void> {
  try {
    const status = await fetchStatus();
    currentStatus = status;

    const container = document.getElementById("projects-container")!;
    container.innerHTML = "";
    knownProjectIds.clear();

    if (status.projects.length === 0) {
      container.innerHTML = '<p class="empty">No projects yet. Add one below.</p>';
    } else {
      for (const p of status.projects) {
        // Sync local state with backend state on initial load
        if (p.manualMode && !localManualMode.has(p.id)) {
          localManualMode.set(p.id, { active: true, startTime: Date.now() - p.elapsedTime });
        }
        container.appendChild(createProjectCard(p));
        knownProjectIds.add(p.id);
      }
    }
  } catch (err) {
    console.error("Failed to load projects:", err);
    document.getElementById("app")!.innerHTML = `
      <div class="error">
        <h2>Failed to load ProTimer</h2>
        <p>Error: ${err}</p>
      </div>
    `;
  }
}

// Fetch data from backend (runs every 5 seconds + on activity log changes)
async function fetchData(): Promise<void> {
  try {
    const status = await fetchStatus();
    lastFetchTime = Date.now();

    // Override backend state with local manual mode state
    for (const p of status.projects) {
      const local = localManualMode.get(p.id);
      if (local) {
        // Local state is the source of truth
        p.manualMode = local.active;
        p.isTracking = local.active || p.claudeState === "active";

        // Cleanup: Only remove local state once backend has caught up
        if (!local.active && !p.manualMode && !p.isTracking) {
          // If we have frozen times, wait for backend to catch up (within 1 second tolerance)
          const frozen = frozenTimes.get(p.id);
          if (frozen) {
            const backendCaughtUp = Math.abs(p.weekTime - frozen.weekTime) < 1000;
            if (backendCaughtUp) {
              localManualMode.delete(p.id);
              frozenTimes.delete(p.id);
            }
          } else {
            // No frozen state, safe to cleanup
            localManualMode.delete(p.id);
          }
        }
      }
    }

    currentStatus = status;

    // Check if projects changed
    const currentIds = new Set(status.projects.map(p => p.id));
    const sameProjects = currentIds.size === knownProjectIds.size &&
      [...currentIds].every(id => knownProjectIds.has(id));

    if (!sameProjects) {
      await rebuildProjects();
      return;
    }

    // Update non-timer elements
    for (const p of status.projects) {
      updateProjectCard(p);
    }

    // Update dock badge
    const isTracking = status.projects.some(p => p.isTracking);
    getCurrentWindow().setBadgeLabel(isTracking ? "●" : undefined).catch(() => {});
  } catch {
    // Ignore fetch errors
  }
}

// Render timers - runs at 60fps but only updates DOM once per second
function renderTimers(): void {
  if (!isAppRunning) return;

  if (!currentStatus) {
    rafId = requestAnimationFrame(renderTimers);
    return;
  }

  const now = Date.now();
  const currentSecond = Math.floor(now / 1000);

  // Only update DOM once per second
  if (currentSecond === lastRenderSecond) {
    requestAnimationFrame(renderTimers);
    return;
  }
  lastRenderSecond = currentSecond;

  let totalWeek = 0;
  let totalToday = 0;
  let totalWeekEarnings = 0;
  let totalTodayEarnings = 0;

  for (const p of currentStatus.projects) {
    let weekTime: number;
    let todayTime: number;

    // Check if we have frozen times (just stopped)
    const frozen = frozenTimes.get(p.id);
    if (frozen) {
      weekTime = frozen.weekTime;
      todayTime = frozen.todayTime;
    } else {
      // Calculate elapsed time for smooth display
      const local = localManualMode.get(p.id);
      let extraTime = 0;

      if (local?.active) {
        // Manual mode: use local start time for instant, accurate tracking
        extraTime = now - local.startTime;
      } else if (p.isTracking && p.elapsedTime > 0) {
        // Auto-tracking: interpolate from backend's elapsed time
        extraTime = p.elapsedTime + (now - lastFetchTime);
      }

      weekTime = p.weekTime + extraTime;
      todayTime = p.todayTime + extraTime;

      // Store what we just calculated so stop button can freeze it
      lastRenderedTimes.set(p.id, { weekTime, todayTime });
    }

    const weekEl = document.getElementById(`week-${p.id}`);
    const todayEl = document.getElementById(`today-${p.id}`);
    const weekEarningsEl = document.getElementById(`week-earnings-${p.id}`);
    const todayEarningsEl = document.getElementById(`today-earnings-${p.id}`);

    if (weekEl) weekEl.textContent = formatDuration(weekTime);
    if (todayEl) todayEl.textContent = formatDuration(todayTime);
    if (weekEarningsEl) weekEarningsEl.textContent = formatEarnings(weekTime, p.hourlyRate);
    if (todayEarningsEl) todayEarningsEl.textContent = formatEarnings(todayTime, p.hourlyRate);

    if (p.hourlyRate) {
      totalWeekEarnings += (weekTime / 3600000) * p.hourlyRate;
      totalTodayEarnings += (todayTime / 3600000) * p.hourlyRate;
    }

    totalWeek += weekTime;
    totalToday += todayTime;
  }

  // Update header stats
  const headerWeekEl = document.getElementById("header-week");
  const headerTodayEl = document.getElementById("header-today");
  const headerWeekEarningsEl = document.getElementById("header-week-earnings");
  const headerTodayEarningsEl = document.getElementById("header-today-earnings");
  if (headerWeekEl) headerWeekEl.textContent = formatDuration(totalWeek);
  if (headerTodayEl) headerTodayEl.textContent = formatDuration(totalToday);
  const hasAnyRate = currentStatus.projects.some(p => p.hourlyRate);
  if (headerWeekEarningsEl) headerWeekEarningsEl.textContent = hasAnyRate ? `$${totalWeekEarnings.toFixed(2)}` : "";
  if (headerTodayEarningsEl) headerTodayEarningsEl.textContent = hasAnyRate ? `$${totalTodayEarnings.toFixed(2)}` : "";

  rafId = requestAnimationFrame(renderTimers);
}

function formatTime(timestamp: number): string {
  const date = new Date(timestamp);
  return date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}


async function showSettingsModal(): Promise<void> {
  const overlay = document.createElement("div");
  overlay.className = "activity-modal-overlay";

  const dataPath = await invoke<string>("get_data_path");
  const businessInfo = await getBusinessInfo();

  overlay.innerHTML = `
    <div class="activity-modal settings-modal">
      <div class="activity-header">
        <h2>Settings</h2>
        <button class="btn-icon btn-close">×</button>
      </div>
      <div class="settings-content">
        <div class="settings-section">
          <h3>Business Information</h3>
          <p class="settings-note">Used for generating invoices</p>
          <form id="business-info-form" class="business-info-form">
            <div class="form-group">
              <label>Business Name</label>
              <input type="text" id="business-name" value="${businessInfo.name}" placeholder="Your Business Name" required />
            </div>
            <div class="form-group">
              <label>Email (optional)</label>
              <input type="email" id="business-email" value="${businessInfo.email ?? ""}" placeholder="billing@yourbusiness.com" />
            </div>
            <div class="form-group">
              <label>Tax Rate (%)</label>
              <input type="number" step="0.01" id="business-tax-rate" value="${businessInfo.taxRate}" placeholder="0" />
            </div>
            <button type="submit" class="btn">Save Business Info</button>
          </form>
        </div>
        <div class="settings-section">
          <h3>Data Location</h3>
          <p class="settings-path">${dataPath}</p>
        </div>
        <div class="settings-section">
          <h3>About</h3>
          <p>ProTimer - Time tracking for hourly engineers</p>
          <p>Local-first, no cloud required</p>
        </div>
      </div>
    </div>
  `;

  document.body.appendChild(overlay);

  const closeModal = () => {
    overlay.remove();
    document.removeEventListener("keydown", handleEscape);
  };

  const handleEscape = (e: KeyboardEvent) => {
    if (e.key === "Escape") closeModal();
  };

  // Handle business info form submission
  overlay.querySelector("#business-info-form")!.addEventListener("submit", async (e) => {
    e.preventDefault();

    const name = (overlay.querySelector("#business-name") as HTMLInputElement).value.trim();
    const emailValue = (overlay.querySelector("#business-email") as HTMLInputElement).value.trim();
    const email = emailValue || null;
    const taxRate = parseFloat((overlay.querySelector("#business-tax-rate") as HTMLInputElement).value) || 0;

    if (!name) {
      alert("Business name is required");
      return;
    }

    try {
      await saveBusinessInfo({ name, email, taxRate });
      alert("Business information saved!");
    } catch (err) {
      alert(`Failed to save: ${err}`);
    }
  });

  document.addEventListener("keydown", handleEscape);
  overlay.querySelector(".btn-close")!.addEventListener("click", closeModal);
  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) closeModal();
  });
}

function showHookSetupModal(): void {
  const overlay = document.createElement("div");
  overlay.className = "activity-modal-overlay";
  overlay.innerHTML = `
    <div class="activity-modal hook-setup-modal">
      <div class="activity-header">
        <h2>Setup Required</h2>
      </div>
      <div class="hook-setup-content">
        <p>ProTimer needs to setup Claude Code hooks to automatically track when Claude is working.</p>
        <p class="hook-setup-details">This will:</p>
        <ul>
          <li>Create a hook script at <code>~/.protimer/hooks/</code></li>
          <li>Update your Claude settings at <code>~/.claude/settings.json</code></li>
        </ul>
        <p class="hook-setup-note">You can still use manual time tracking without hooks.</p>
      </div>
      <div class="hook-setup-buttons">
        <button class="btn btn-skip" id="btn-skip-hooks">Skip for Now</button>
        <button class="btn btn-install" id="btn-install-hooks">Install Hooks</button>
      </div>
    </div>
  `;

  document.body.appendChild(overlay);

  const timeoutIds: number[] = [];

  const closeModal = () => {
    timeoutIds.forEach(id => clearTimeout(id));
    overlay.remove();
  };

  overlay.querySelector("#btn-skip-hooks")!.addEventListener("click", closeModal);

  overlay.querySelector("#btn-install-hooks")!.addEventListener("click", async () => {
    const btn = overlay.querySelector("#btn-install-hooks") as HTMLButtonElement;
    btn.textContent = "Installing...";
    btn.disabled = true;

    try {
      const status = await installHooks();
      if (status.fullyInstalled) {
        btn.textContent = "Installed!";
        timeoutIds.push(window.setTimeout(closeModal, 1000));
      } else {
        btn.textContent = "Partial Install";
        timeoutIds.push(window.setTimeout(() => {
          if (document.body.contains(overlay)) {
            btn.textContent = "Install Hooks";
            btn.disabled = false;
          }
        }, 2000));
      }
    } catch (err) {
      btn.textContent = "Error";
      console.error("Failed to install hooks:", err);
      timeoutIds.push(window.setTimeout(() => {
        if (document.body.contains(overlay)) {
          btn.textContent = "Install Hooks";
          btn.disabled = false;
        }
      }, 2000));
    }
  });
}

async function checkAndShowHookSetup(): Promise<void> {
  try {
    const status = await checkHooksInstalled();
    if (!status.fullyInstalled) {
      showHookSetupModal();
    }
  } catch (err) {
    console.error("Failed to check hooks:", err);
  }
}

function parseTimeInput(timeStr: string, referenceDate: Date): number | null {
  // Parse time like "10:30 AM" or "2:45 PM"
  const match = timeStr.match(/^(\d{1,2}):(\d{2})\s*(AM|PM)?$/i);
  if (!match) return null;

  let hours = parseInt(match[1], 10);
  const minutes = parseInt(match[2], 10);
  const meridiem = match[3]?.toUpperCase();

  if (minutes < 0 || minutes > 59) return null;

  if (meridiem) {
    if (hours < 1 || hours > 12) return null;
    if (meridiem === "PM" && hours !== 12) hours += 12;
    if (meridiem === "AM" && hours === 12) hours = 0;
  } else {
    if (hours < 0 || hours > 23) return null;
  }

  const result = new Date(referenceDate);
  result.setHours(hours, minutes, 0, 0);
  return result.getTime();
}

function formatTimeForInput(timestamp: number): string {
  const date = new Date(timestamp);
  return date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

async function showActivityModal(project: Project): Promise<void> {
  // Day navigation state
  const getDayStart = (date: Date): number => {
    const d = new Date(date);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  };

  const isToday = (dayStart: number): boolean => {
    return dayStart === getDayStart(new Date());
  };

  const formatDayLabel = (dayStart: number): string => {
    if (isToday(dayStart)) return "Today";
    const d = new Date(dayStart);
    const yesterday = new Date();
    yesterday.setDate(yesterday.getDate() - 1);
    yesterday.setHours(0, 0, 0, 0);
    if (dayStart === yesterday.getTime()) return "Yesterday";
    return d.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
  };

  let currentDayStart = getDayStart(new Date());
  let entries = await fetchEntries(project.id, currentDayStart);

  const overlay = document.createElement("div");
  overlay.className = "activity-modal-overlay";

  const renderEntries = (entryList: TimeEntry[]) => {
    if (entryList.length === 0) {
      return '<p class="empty">No time entries for this day.</p>';
    }
    return entryList
      .sort((a, b) => b.startTime - a.startTime)
      .map(e => {
        const duration = e.endTime - e.startTime;
        return `
        <div class="entry" data-entry-id="${e.id}" data-start="${e.startTime}" data-end="${e.endTime}">
          <div class="entry-info">
            <span class="entry-time" title="Click to edit">
              <span class="time-display">${formatTime(e.startTime)} - ${formatTime(e.endTime)}</span>
              <span class="time-edit" style="display: none;">
                <input type="text" class="time-input time-start" value="${formatTimeForInput(e.startTime)}" />
                <span class="time-separator">-</span>
                <input type="text" class="time-input time-end" value="${formatTimeForInput(e.endTime)}" />
              </span>
            </span>
            <span class="entry-duration">${formatDuration(duration)}</span>
          </div>
          <button class="btn-icon btn-delete-entry" title="Delete entry">×</button>
        </div>
      `;
      }).join("");
  };

  const totalTime = entries.reduce((sum, e) => sum + (e.endTime - e.startTime), 0);

  overlay.innerHTML = `
    <div class="activity-modal">
      <div class="activity-header">
        <h2>${project.name}</h2>
        <button class="btn-icon btn-close">×</button>
      </div>
      <div class="day-nav">
        <button class="btn-icon btn-day-prev">‹</button>
        <span class="day-nav-label">${formatDayLabel(currentDayStart)}</span>
        <button class="btn-icon btn-day-next" ${isToday(currentDayStart) ? 'disabled' : ''}>›</button>
      </div>
      <div class="activity-summary">
        <span>${formatDuration(totalTime)}</span>
        <span>${entries.length} entries</span>
      </div>
      <div class="add-time-form" style="display: none;">
        <div class="add-time-inputs">
          <input type="text" class="time-input add-time-start" placeholder="Start" />
          <span class="time-separator">-</span>
          <input type="text" class="time-input add-time-end" placeholder="End" />
        </div>
        <div class="add-time-hint">Format: 9:00 AM or 14:30</div>
        <div class="add-time-buttons">
          <button class="btn btn-cancel-add">Cancel</button>
          <button class="btn btn-save-add">Add</button>
        </div>
      </div>
      <button class="btn btn-add-time" id="btn-add-time">+ Add Time</button>
      <div class="activity-entries">
        ${renderEntries(entries)}
      </div>
    </div>
  `;

  document.body.appendChild(overlay);

  const closeModal = () => {
    overlay.remove();
    document.removeEventListener("keydown", handleEscape);
  };

  const handleEscape = (e: KeyboardEvent) => {
    if (e.key === "Escape") closeModal();
  };

  document.addEventListener("keydown", handleEscape);
  overlay.querySelector(".btn-close")!.addEventListener("click", closeModal);
  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) closeModal();
  });

  // Day navigation
  const dayLabel = overlay.querySelector(".day-nav-label") as HTMLElement;
  const prevBtn = overlay.querySelector(".btn-day-prev") as HTMLButtonElement;
  const nextBtn = overlay.querySelector(".btn-day-next") as HTMLButtonElement;

  const navigateToDay = async (dayStart: number) => {
    currentDayStart = dayStart;
    entries = await fetchEntries(project.id, currentDayStart);
    dayLabel.textContent = formatDayLabel(currentDayStart);
    nextBtn.disabled = isToday(currentDayStart);
    entriesContainer.innerHTML = renderEntries(entries);
    updateSummary();
    hideAddForm();
    setupEntryEventHandlers();
  };

  prevBtn.addEventListener("click", () => {
    navigateToDay(currentDayStart - 86_400_000);
  });

  nextBtn.addEventListener("click", () => {
    if (!isToday(currentDayStart)) {
      navigateToDay(currentDayStart + 86_400_000);
    }
  });

  // Setup add time functionality
  const addTimeBtn = overlay.querySelector("#btn-add-time") as HTMLButtonElement;
  const addTimeForm = overlay.querySelector(".add-time-form") as HTMLElement;
  const addTimeStart = overlay.querySelector(".add-time-start") as HTMLInputElement;
  const addTimeEnd = overlay.querySelector(".add-time-end") as HTMLInputElement;
  const cancelAddBtn = overlay.querySelector(".btn-cancel-add") as HTMLButtonElement;
  const saveAddBtn = overlay.querySelector(".btn-save-add") as HTMLButtonElement;
  const summaryEl = overlay.querySelector(".activity-summary") as HTMLElement;
  const entriesContainer = overlay.querySelector(".activity-entries") as HTMLElement;

  const updateSummary = () => {
    const total = entries.reduce((sum, e) => sum + (e.endTime - e.startTime), 0);
    summaryEl.innerHTML = `
      <span>${formatDuration(total)}</span>
      <span>${entries.length} entries</span>
    `;
  };

  const showAddForm = () => {
    addTimeBtn.style.display = "none";
    addTimeForm.style.display = "block";
    addTimeStart.value = "";
    addTimeEnd.value = "";
    addTimeStart.focus();
  };

  const hideAddForm = () => {
    addTimeForm.style.display = "none";
    addTimeBtn.style.display = "block";
  };

  const saveNewEntry = async () => {
    const referenceDay = new Date(currentDayStart);

    const startTime = parseTimeInput(addTimeStart.value.trim(), referenceDay);
    const endTime = parseTimeInput(addTimeEnd.value.trim(), referenceDay);

    if (startTime === null || endTime === null || startTime >= endTime) {
      addTimeStart.classList.add("error");
      addTimeEnd.classList.add("error");
      setTimeout(() => {
        addTimeStart.classList.remove("error");
        addTimeEnd.classList.remove("error");
      }, 1000);
      return;
    }

    try {
      const newEntry = await addTimeEntry(project.id, startTime, endTime);
      entries.push(newEntry);
      entriesContainer.innerHTML = renderEntries(entries);
      updateSummary();
      hideAddForm();
      setupEntryEventHandlers();
    } catch (err) {
      console.error("Failed to add entry:", err);
    }
  };

  addTimeBtn.addEventListener("click", showAddForm);
  cancelAddBtn.addEventListener("click", hideAddForm);
  saveAddBtn.addEventListener("click", saveNewEntry);

  [addTimeStart, addTimeEnd].forEach(input => {
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        saveNewEntry();
      } else if (e.key === "Escape") {
        e.preventDefault();
        hideAddForm();
      }
    });
  });

  // Setup inline editing and delete handlers for entries
  const setupEntryEventHandlers = () => {
    overlay.querySelectorAll(".entry-time").forEach(timeEl => {
      const display = timeEl.querySelector(".time-display") as HTMLElement;
      const edit = timeEl.querySelector(".time-edit") as HTMLElement;
      const startInput = timeEl.querySelector(".time-start") as HTMLInputElement;
      const endInput = timeEl.querySelector(".time-end") as HTMLInputElement;
      const entryEl = timeEl.closest(".entry") as HTMLElement;

      const enterEditMode = () => {
        display.style.display = "none";
        edit.style.display = "inline-flex";
        startInput.focus();
        startInput.select();
      };

      const exitEditMode = async (save: boolean) => {
        if (save) {
          const entryId = entryEl.dataset.entryId!;
          const originalStart = parseInt(entryEl.dataset.start!);
          const originalEnd = parseInt(entryEl.dataset.end!);
          const referenceDate = new Date(originalStart);

          const newStart = parseTimeInput(startInput.value.trim(), referenceDate);
          const newEnd = parseTimeInput(endInput.value.trim(), referenceDate);

          if (newStart !== null && newEnd !== null && newStart < newEnd) {
            try {
              await updateEntry(entryId, newStart, newEnd);
              // Update the display and data attributes
              entryEl.dataset.start = String(newStart);
              entryEl.dataset.end = String(newEnd);
              display.textContent = `${formatTime(newStart)} - ${formatTime(newEnd)}`;
              startInput.value = formatTimeForInput(newStart);
              endInput.value = formatTimeForInput(newEnd);
              // Update duration
              const durationEl = entryEl.querySelector(".entry-duration");
              if (durationEl) {
                durationEl.textContent = formatDuration(newEnd - newStart);
              }
              // Update local entries array
              const entry = entries.find(e => e.id === entryId);
              if (entry) {
                entry.startTime = newStart;
                entry.endTime = newEnd;
                updateSummary();
              }
            } catch (err) {
              console.error("Failed to update entry:", err);
              // Revert inputs to original values
              startInput.value = formatTimeForInput(originalStart);
              endInput.value = formatTimeForInput(originalEnd);
            }
          } else {
            // Invalid time, revert
            startInput.value = formatTimeForInput(originalStart);
            endInput.value = formatTimeForInput(originalEnd);
          }
        }

        display.style.display = "inline";
        edit.style.display = "none";
      };

      display.addEventListener("click", (e) => {
        e.stopPropagation();
        enterEditMode();
      });

      [startInput, endInput].forEach(input => {
        input.addEventListener("blur", () => {
          // Small delay to allow clicking the other input
          setTimeout(() => {
            if (!edit.contains(document.activeElement)) {
              exitEditMode(true);
            }
          }, 100);
        });

        input.addEventListener("keydown", (e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            exitEditMode(true);
          } else if (e.key === "Escape") {
            e.preventDefault();
            const originalStart = parseInt(entryEl.dataset.start!);
            const originalEnd = parseInt(entryEl.dataset.end!);
            startInput.value = formatTimeForInput(originalStart);
            endInput.value = formatTimeForInput(originalEnd);
            exitEditMode(false);
          } else if (e.key === "Tab" && !e.shiftKey && input === endInput) {
            e.preventDefault();
            exitEditMode(true);
          }
        });

        input.addEventListener("click", (e) => e.stopPropagation());
      });
    });

    overlay.querySelectorAll(".btn-delete-entry").forEach(btn => {
      btn.addEventListener("click", async (e) => {
        e.stopPropagation();
        const entryEl = (btn as HTMLElement).closest(".entry")!;
        const entryId = (entryEl as HTMLElement).dataset.entryId!;
        showConfirmDialog("Delete this entry?", async () => {
          await deleteEntry(entryId);
          entries = entries.filter(e => e.id !== entryId);
          entryEl.remove();
          updateSummary();
          if (entries.length === 0) {
            entriesContainer.innerHTML = '<p class="empty">No time entries yet.</p>';
          }
        });
      });
    });
  };

  setupEntryEventHandlers();
}

function showConfirmDialog(message: string, onConfirm: () => void): void {
  const overlay = document.createElement("div");
  overlay.className = "confirm-dialog-overlay";
  overlay.innerHTML = `
    <div class="confirm-dialog">
      <p>${message}</p>
      <div class="confirm-buttons">
        <button class="btn btn-cancel">Cancel</button>
        <button class="btn btn-confirm">Delete</button>
      </div>
    </div>
  `;

  document.body.appendChild(overlay);

  const handleEscape = (e: KeyboardEvent) => {
    if (e.key === "Escape") closeDialog();
  };

  const closeDialog = () => {
    document.removeEventListener("keydown", handleEscape);
    overlay.remove();
  };

  overlay.querySelector(".btn-cancel")!.addEventListener("click", closeDialog);
  overlay.querySelector(".btn-confirm")!.addEventListener("click", () => {
    closeDialog();
    onConfirm();
  });
  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) closeDialog();
  });

  document.addEventListener("keydown", handleEscape);
}

function showRateDialog(project: Project): void {
  const overlay = document.createElement("div");
  overlay.className = "confirm-dialog-overlay";
  overlay.innerHTML = `
    <div class="confirm-dialog rate-dialog">
      <h3>Hourly Rate</h3>
      <p>${project.name}</p>
      <div class="rate-input-row">
        <span class="rate-currency">$</span>
        <input type="number" class="rate-input" value="${project.hourlyRate ?? ""}" placeholder="0" min="0" step="0.01" />
        <span class="rate-suffix">/hr</span>
      </div>
      <div class="confirm-buttons">
        <button class="btn btn-cancel">Cancel</button>
        <button class="btn btn-confirm">Save</button>
      </div>
    </div>
  `;

  document.body.appendChild(overlay);

  const input = overlay.querySelector(".rate-input") as HTMLInputElement;
  input.focus();
  input.select();

  const handleKeydown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      closeDialog();
    } else if (e.key === "Enter") {
      overlay.querySelector<HTMLButtonElement>(".btn-confirm")!.click();
    }
  };

  const closeDialog = () => {
    document.removeEventListener("keydown", handleKeydown);
    overlay.remove();
  };

  overlay.querySelector(".btn-cancel")!.addEventListener("click", closeDialog);
  overlay.querySelector(".btn-confirm")!.addEventListener("click", async () => {
    const value = input.value.trim();
    const rate = value === "" ? null : parseFloat(value);
    await updateProjectRate(project.id, rate);
    closeDialog();
    rebuildProjects();
  });
  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) closeDialog();
  });

  document.addEventListener("keydown", handleKeydown);
}

function showRenameDialog(project: Project): void {
  const overlay = document.createElement("div");
  overlay.className = "confirm-dialog-overlay";
  overlay.innerHTML = `
    <div class="confirm-dialog">
      <h3>Rename Project</h3>
      <input type="text" class="rename-input" value="${project.name}" placeholder="Project name" />
      <div class="confirm-buttons">
        <button class="btn btn-cancel">Cancel</button>
        <button class="btn btn-confirm">Save</button>
      </div>
    </div>
  `;

  document.body.appendChild(overlay);

  const input = overlay.querySelector(".rename-input") as HTMLInputElement;
  input.focus();
  input.select();

  const handleKeydown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      closeDialog();
    } else if (e.key === "Enter") {
      overlay.querySelector<HTMLButtonElement>(".btn-confirm")!.click();
    }
  };

  const closeDialog = () => {
    document.removeEventListener("keydown", handleKeydown);
    overlay.remove();
  };

  overlay.querySelector(".btn-cancel")!.addEventListener("click", closeDialog);
  overlay.querySelector(".btn-confirm")!.addEventListener("click", async () => {
    const newName = input.value.trim();
    if (!newName) {
      alert("Project name cannot be empty");
      return;
    }
    await updateProjectName(project.id, newName);
    closeDialog();
    rebuildProjects();
  });
  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) closeDialog();
  });

  document.addEventListener("keydown", handleKeydown);
}

function showInvoiceDialog(project: Project): void {
  const overlay = document.createElement("div");
  overlay.className = "confirm-dialog-overlay";

  // Calculate last Monday to most recent Sunday
  const today = new Date();
  const dayOfWeek = today.getDay(); // 0 = Sunday, 1 = Monday, ..., 6 = Saturday

  // Days since last Monday
  const daysSinceMonday = (dayOfWeek + 6) % 7; // Mon=0, Tue=1, ..., Sun=6

  // Last Monday
  const lastMonday = new Date(today);
  lastMonday.setDate(today.getDate() - daysSinceMonday - 7); // Go back a week from current week
  lastMonday.setHours(0, 0, 0, 0);

  // Most recent Sunday (last Sunday)
  const lastSunday = new Date(lastMonday);
  lastSunday.setDate(lastMonday.getDate() + 6);
  lastSunday.setHours(0, 0, 0, 0); // Start of Sunday, not end

  const formatDate = (d: Date) => {
    const year = d.getFullYear();
    const month = String(d.getMonth() + 1).padStart(2, '0');
    const day = String(d.getDate()).padStart(2, '0');
    return `${year}-${month}-${day}`;
  };

  overlay.innerHTML = `
    <div class="confirm-dialog invoice-dialog">
      <h3>Generate Invoice</h3>
      <p>${project.name}</p>
      <div class="form-group">
        <label>Start Date</label>
        <input type="date" id="invoice-start-date" value="${formatDate(lastMonday)}" />
      </div>
      <div class="form-group">
        <label>End Date</label>
        <input type="date" id="invoice-end-date" value="${formatDate(lastSunday)}" />
      </div>
      <div class="form-group">
        <label>Extra Hours (tracked outside ProTimer)</label>
        <input type="number" id="invoice-extra-hours" value="0" min="0" step="0.01" placeholder="0.00" />
      </div>
      <p class="invoice-note">Invoice will be saved to ~/.protimer/invoices/</p>
      <div class="confirm-buttons">
        <button class="btn btn-cancel">Cancel</button>
        <button class="btn btn-confirm">Generate PDF</button>
      </div>
    </div>
  `;

  document.body.appendChild(overlay);

  const closeDialog = () => {
    overlay.remove();
  };

  overlay.querySelector(".btn-cancel")!.addEventListener("click", closeDialog);
  overlay.querySelector(".btn-confirm")!.addEventListener("click", async () => {
    const startDate = (overlay.querySelector("#invoice-start-date") as HTMLInputElement).value;
    const endDate = (overlay.querySelector("#invoice-end-date") as HTMLInputElement).value;
    const extraHoursInput = (overlay.querySelector("#invoice-extra-hours") as HTMLInputElement).value;

    if (!startDate || !endDate) {
      alert("Please select both start and end dates");
      return;
    }

    const extraHours = parseFloat(extraHoursInput) || 0;
    if (extraHours < 0) {
      alert("Extra hours cannot be negative");
      return;
    }

    // Parse dates in local timezone (not UTC) to avoid off-by-one day errors
    const [startYear, startMonth, startDay] = startDate.split('-').map(Number);
    const [endYear, endMonth, endDay] = endDate.split('-').map(Number);
    const startMs = new Date(startYear, startMonth - 1, startDay, 0, 0, 0, 0).getTime();
    const endMs = new Date(endYear, endMonth - 1, endDay, 23, 59, 59, 999).getTime();

    try {
      const pdfPath = await generateInvoice(project.id, startMs, endMs, extraHours);
      // Auto-open the generated invoice
      await invoke("open_file", { filePath: pdfPath });
      alert(`Invoice generated and opened!\n\nSaved to: ${pdfPath}`);
      closeDialog();
    } catch (err) {
      alert(`Failed to generate invoice: ${err}`);
    }
  });
  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) closeDialog();
  });
}

function showIdleDialog(idleTime: number, trackingProjects: Project[]): void {
  idleDialogOpen = true;
  const idleTimeFormatted = formatDuration(idleTime);
  const projectNames = trackingProjects.map(p => p.name).join(", ");

  const dialog = document.createElement("div");
  dialog.className = "idle-dialog-overlay";
  dialog.innerHTML = `
    <div class="idle-dialog">
      <h2>Idle Detected</h2>
      <p>You've been idle for <strong>${idleTimeFormatted}</strong></p>
      <p class="idle-projects">Active timers: ${projectNames}</p>
      <div class="idle-options">
        <label class="idle-option">
          <input type="radio" name="idle-action" value="keep">
          <span>Keep idle time</span>
        </label>
        <label class="idle-option">
          <input type="radio" name="idle-action" value="discard" checked>
          <span>Discard idle time</span>
        </label>
      </div>
      <div class="idle-buttons">
        <button class="btn idle-btn-continue">Continue</button>
        <button class="btn idle-btn-stop">Stop All</button>
      </div>
    </div>
  `;

  document.body.appendChild(dialog);

  dialog.querySelector(".idle-btn-continue")?.addEventListener("click", () => {
    idleDialogOpen = false;
    dialog.remove();
  });

  dialog.querySelector(".idle-btn-stop")?.addEventListener("click", () => {
    const discardIdle = (dialog.querySelector('input[name="idle-action"]:checked') as HTMLInputElement)?.value === "discard";
    const stopTrackingPromises: Promise<void>[] = [];

    for (const project of trackingProjects) {
      // Update local state immediately for instant UI
      localManualMode.set(project.id, { active: false, startTime: 0 });
      // Background sync to backend
      if (discardIdle) {
        const idleStartTime = Date.now() - idleTime;
        stopTrackingPromises.push(stopTracking(project.id, idleStartTime));
      } else {
        stopTrackingPromises.push(stopTracking(project.id));
      }
    }

    // Wait for all stops to complete, then refresh totals
    Promise.all(stopTrackingPromises).then(() => fetchData());

    // Update UI immediately from local state
    if (currentStatus) {
      for (const project of trackingProjects) {
        const p = currentStatus.projects.find(proj => proj.id === project.id);
        if (p) {
          p.manualMode = false;
          p.isTracking = p.claudeState === "active";
          updateProjectCard(p);
        }
      }
    }

    idleDialogOpen = false;
    dialog.remove();
  });
}

function checkIdle(): void {
  if (!currentStatus || idleDialogOpen) return;

  const trackingProjects = currentStatus.projects.filter(p => p.isTracking);
  if (trackingProjects.length === 0) return;

  if (currentStatus.systemIdleTime >= IDLE_THRESHOLD_MS) {
    showIdleDialog(currentStatus.systemIdleTime, trackingProjects);
  }
}

// Cleanup function for window unload
function cleanup(): void {
  isAppRunning = false;
  if (fetchDataIntervalId !== null) {
    clearInterval(fetchDataIntervalId);
    fetchDataIntervalId = null;
  }
  if (checkIdleIntervalId !== null) {
    clearInterval(checkIdleIntervalId);
    checkIdleIntervalId = null;
  }
  if (rafId !== null) {
    cancelAnimationFrame(rafId);
    rafId = null;
  }
}

// Initialize
buildShell();
rebuildProjects();
checkAndShowHookSetup();

// Listen for activity log changes from the file watcher
listen("activity-log-changed", () => {
  fetchData();
});

// Fetch data every 5 seconds (reduced from 1 second since we have file watcher)
fetchDataIntervalId = window.setInterval(fetchData, 5000);

// Render timers at 60fps (updates DOM once per second)
rafId = requestAnimationFrame(renderTimers);

// Check for idle
checkIdleIntervalId = window.setInterval(checkIdle, 10000);

// Cleanup on window unload to prevent memory leaks
window.addEventListener("beforeunload", cleanup);
