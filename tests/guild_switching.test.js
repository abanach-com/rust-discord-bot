const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

async function flushTicks(n = 30) {
  for (let i = 0; i < n; i++) {
    await new Promise((r) => setImmediate(r));
  }
}

function createMockElement(tag = "div") {
  const children = [];
  const classList = new Set();
  const attributes = {};
  const eventListeners = {};
  const style = { display: "none" };

  const el = {
    tagName: tag.toUpperCase(),
    dataset: {},
    style,
    value: "",
    checked: false,
    disabled: false,
    textContent: "",
    get innerHTML() {
      return el._html || "";
    },
    set innerHTML(val) {
      el._html = val;
      children.length = 0;
      const classRegex = /class="([^"]+)"/g;
      let match;
      while ((match = classRegex.exec(val)) !== null) {
        const child = createMockElement("div");
        child.className = match[1];
        children.push(child);
      }
    },
    children,
    replaceChildren(...newChildren) {
      children.length = 0;
      for (const c of newChildren) {
        if (c) children.push(c);
      }
    },
    append(...newChildren) {
      for (const c of newChildren) {
        if (c) children.push(c);
      }
    },
    remove() {
      // noop
    },
    get className() {
      return Array.from(classList).join(" ");
    },
    set className(val) {
      classList.clear();
      if (val) {
        val.split(/\s+/).forEach((c) => {
          if (c) classList.add(c);
        });
      }
    },
    classList: {
      add: (cls) => classList.add(cls),
      remove: (cls) => classList.delete(cls),
      toggle: (cls, force) => {
        if (force === undefined) {
          if (classList.has(cls)) classList.delete(cls);
          else classList.add(cls);
        } else if (force) {
          classList.add(cls);
        } else {
          classList.delete(cls);
        }
      },
      contains: (cls) => classList.has(cls),
    },
    setAttribute: (name, val) => {
      attributes[name] = String(val);
    },
    getAttribute: (name) => attributes[name] || null,
    addEventListener: (evt, fn) => {
      if (!eventListeners[evt]) eventListeners[evt] = [];
      eventListeners[evt].push(fn);
    },
    trigger: async (evt, data = {}) => {
      const handlers = eventListeners[evt] || [];
      for (const h of handlers) {
        await h({ target: el, preventDefault: () => {}, ...data });
      }
    },
    querySelector: (selector) => el._find(selector),
    querySelectorAll: (selector) => {
      const res = [];
      el._findAll(selector, res);
      return res;
    },
    _find: (selector) => {
      if (el._matches(selector)) return el;
      for (const child of children) {
        if (child._matches && child._matches(selector)) return child;
        const sub = child._find ? child._find(selector) : null;
        if (sub) return sub;
      }
      return null;
    },
    _findAll: (selector, list) => {
      if (el._matches(selector)) list.push(el);
      for (const child of children) {
        if (child._matches && child._matches(selector)) list.push(child);
        if (child._findAll) child._findAll(selector, list);
      }
    },
    _matches: (selector) => {
      if (selector.startsWith(".")) {
        return classList.has(selector.slice(1));
      }
      if (selector.startsWith("#")) {
        return attributes["id"] === selector.slice(1);
      }
      if (selector.startsWith("[data-module-id=")) {
        const id = selector.slice(17, -2);
        return el.dataset.moduleId === id;
      }
      return false;
    },
    cloneNode: () => {
      const copy = createMockElement(tag);
      copy.className = el.className;
      copy.dataset = { ...el.dataset };
      for (const [k, v] of Object.entries(attributes)) {
        copy.setAttribute(k, v);
      }
      for (const c of children) {
        copy.append(c.cloneNode());
      }
      return copy;
    },
  };

  return el;
}

function createTestHarness(fetchHandler) {
  const queryMap = {
    "#login-view": createMockElement("div"),
    "#dashboard-view": createMockElement("div"),
    "#account": createMockElement("div"),
    "#account-name": createMockElement("span"),
    "#account-id": createMockElement("span"),
    "#guild-select": createMockElement("select"),
    "#modules": createMockElement("section"),
    "#module-count": createMockElement("span"),
    "#save-status": createMockElement("span"),
    "#logout-button": createMockElement("button"),
    "#nav-modules": createMockElement("a"),
    "#nav-moderation": createMockElement("a"),
    "#nav-audit": createMockElement("a"),
    "#nav-analytics": createMockElement("a"),
    "#view-modules": createMockElement("div"),
    "#view-moderation": createMockElement("div"),
    "#view-audit": createMockElement("div"),
    "#view-analytics": createMockElement("div"),
    "#cases-list": createMockElement("div"),
    "#audit-list": createMockElement("div"),
    "#toast-container": createMockElement("div"),
    "#kpi-active-modules": createMockElement("span"),
    "#kpi-modules-hint": createMockElement("span"),
    "#kpi-total-cases": createMockElement("span"),
    "#kpi-audit-events": createMockElement("span"),
    "#kpi-bot-status": createMockElement("span"),
    "#kpi-uptime": createMockElement("span"),
    "#void-dialog": createMockElement("dialog"),
    "#void-form": createMockElement("form"),
    "#void-reason-input": createMockElement("textarea"),
    "#void-cancel-btn": createMockElement("button"),
    "#analytics-health-donut": createMockElement("div"),
    "#analytics-requests-sparkline": createMockElement("div"),
    "#analytics-details": createMockElement("div"),
  };

  const template = createMockElement("template");
  const templateContent = createMockElement("div");
  const card = createMockElement("article");
  card.classList.add("module-card");

  const cat = createMockElement("span");
  cat.classList.add("module-category");
  const name = createMockElement("h2");
  name.classList.add("module-name");
  const desc = createMockElement("p");
  desc.classList.add("module-description");
  const mid = createMockElement("span");
  mid.classList.add("module-id");
  const mver = createMockElement("span");
  mver.classList.add("module-version");
  const mstate = createMockElement("span");
  mstate.classList.add("module-state");
  const enabled = createMockElement("input");
  enabled.classList.add("module-enabled");
  enabled.type = "checkbox";
  const config = createMockElement("textarea");
  config.classList.add("module-config");
  const saveBtn = createMockElement("button");
  saveBtn.classList.add("save-button");
  const formContainer = createMockElement("div");
  formContainer.classList.add("module-form-container");
  const jsonDetails = createMockElement("details");
  jsonDetails.classList.add("json-details");

  card.append(cat, name, desc, mid, mver, mstate, enabled, config, formContainer, jsonDetails, saveBtn);
  templateContent.append(card);
  template.content = templateContent;
  queryMap["#module-template"] = template;

  const mockDocument = {
    querySelector: (sel) => queryMap[sel] || null,
    querySelectorAll: (sel) => (queryMap[sel] ? [queryMap[sel]] : []),
    createElement: (tag) => createMockElement(tag),
    createTextNode: (txt) => ({ textContent: txt }),
  };

  class MockEventSource {
    constructor(url) {
      this.url = url;
      this.listeners = {};
      MockEventSource.instances.push(this);
    }
    addEventListener(evt, fn) {
      if (!this.listeners[evt]) this.listeners[evt] = [];
      this.listeners[evt].push(fn);
    }
    close() {
      this.closed = true;
    }
  }
  MockEventSource.instances = [];

  const sandbox = {
    document: mockDocument,
    EventSource: MockEventSource,
    fetch: fetchHandler,
    Headers: globalThis.Headers,
    AbortController: globalThis.AbortController,
    AbortSignal: globalThis.AbortSignal,
    Intl: globalThis.Intl,
    setTimeout: (fn) => fn(),
    setInterval: () => 123,
    clearInterval: () => {},
    console: { log: () => {}, warn: () => {}, error: () => {} },
  };

  const appJsCode = fs.readFileSync(path.join(__dirname, "../web/static/app.js"), "utf8");
  const augmentedCode = appJsCode + "\nglobalThis.__app = { state, ui };";
  vm.createContext(sandbox);
  vm.runInContext(augmentedCode, sandbox);

  return { sandbox, queryMap, MockEventSource };
}

test("Delayed response for Guild A does NOT render into Guild B after fast guild switch", async () => {
  let resolveGuildAModules;
  const guildAPromise = new Promise((resolve) => {
    resolveGuildAModules = resolve;
  });

  const harness = createTestHarness(async (url, opts = {}) => {
    if (url === "/api/session") {
      return {
        ok: true,
        status: 200,
        json: async () => ({
          user: { id: "1001", username: "Tester" },
          guilds: [
            { id: "guild-a", name: "Guild A", permissions: "32" },
            { id: "guild-b", name: "Guild B", permissions: "32" },
          ],
          csrf_token: "csrf-token-123",
        }),
      };
    }
    if (url === "/health/live") {
      return {
        ok: true,
        status: 200,
        json: async () => ({ status: "ok", uptime_seconds: 100, version: "0.1.0" }),
      };
    }
    if (url === "/api/guilds/guild-a/modules") {
      return await guildAPromise;
    }
    if (url === "/api/guilds/guild-b/modules") {
      return {
        ok: true,
        status: 200,
        json: async () => [
          {
            manifest: { id: "mod_b", name: "Module B", category: "General", description: "B", version: "1.0" },
            enabled: true,
            configured: true,
            updated_at: "2026-11-02T19:00:00Z",
            config: {},
          },
        ],
      };
    }
    return {
      ok: true,
      status: 200,
      json: async () => ({}),
    };
  });

  await flushTicks();

  assert.equal(harness.sandbox.__app.state.guildId, "guild-a");

  // Fast switch to Guild B while Guild A modules are still pending
  const guildSelect = harness.queryMap["#guild-select"];
  await guildSelect.trigger("change", { target: { value: "guild-b" } });

  assert.equal(harness.sandbox.__app.state.guildId, "guild-b");

  await flushTicks();

  const modulesContainer = harness.queryMap["#modules"];
  assert.equal(modulesContainer.children.length, 1);
  const cardB = modulesContainer.children[0];
  const renderedName = cardB.querySelector(".module-name");
  assert.equal(renderedName.textContent, "Module B");

  // Now resolve Guild A's delayed response
  resolveGuildAModules({
    ok: true,
    status: 200,
    json: async () => [
      {
        manifest: { id: "mod_a", name: "Module A", category: "General", description: "A", version: "1.0" },
        enabled: true,
        configured: true,
        updated_at: "2026-11-02T18:00:00Z",
        config: {},
      },
    ],
  });

  await flushTicks();

  // Verify that Guild A's delayed response did NOT overwrite Guild B's modules!
  assert.equal(modulesContainer.children.length, 1);
  assert.equal(modulesContainer.children[0].querySelector(".module-name").textContent, "Module B");
  assert.equal(harness.sandbox.__app.state.modulesData[0].manifest.id, "mod_b");
});

test("Switching guild aborts in-flight fetch requests via AbortController", async () => {
  let guildASignal;
  const harness = createTestHarness(async (url, opts = {}) => {
    if (url === "/api/session") {
      return {
        ok: true,
        status: 200,
        json: async () => ({
          user: { id: "1001", username: "Tester" },
          guilds: [
            { id: "guild-a", name: "Guild A", permissions: "32" },
            { id: "guild-b", name: "Guild B", permissions: "32" },
          ],
          csrf_token: "token",
        }),
      };
    }
    if (url === "/api/guilds/guild-a/modules") {
      guildASignal = opts.signal;
      return new Promise(() => {});
    }
    return {
      ok: true,
      status: 200,
      json: async () => ({}),
    };
  });

  await flushTicks();

  assert.ok(guildASignal, "Guild A fetch should receive an abort signal");
  assert.equal(guildASignal.aborted, false, "Guild A signal should be active initially");

  // Switch to Guild B
  const guildSelect = harness.queryMap["#guild-select"];
  await guildSelect.trigger("change", { target: { value: "guild-b" } });

  assert.equal(guildASignal.aborted, true, "Guild A in-flight signal must be aborted upon guild change");
});

test("Saving a module from Guild A is rejected and NEVER directs save to Guild B after guild change", async () => {
  const puts = [];

  const harness = createTestHarness(async (url, opts = {}) => {
    if (url === "/api/session") {
      return {
        ok: true,
        status: 200,
        json: async () => ({
          user: { id: "1001", username: "Tester" },
          guilds: [
            { id: "guild-a", name: "Guild A", permissions: "32" },
            { id: "guild-b", name: "Guild B", permissions: "32" },
          ],
          csrf_token: "token",
        }),
      };
    }
    if (url === "/api/guilds/guild-a/modules") {
      return {
        ok: true,
        status: 200,
        json: async () => [
          {
            manifest: { id: "mod_a", name: "Module A", category: "General", description: "A", version: "1.0" },
            enabled: true,
            configured: false,
            config: { key: "value_a" },
          },
        ],
      };
    }
    if (opts.method === "PUT") {
      puts.push({ url, body: opts.body });
      return {
        ok: true,
        status: 200,
        json: async () => ({ updated_at: "2026-11-02T20:00:00Z" }),
      };
    }
    return {
      ok: true,
      status: 200,
      json: async () => ({}),
    };
  });

  await flushTicks();

  const modulesContainer = harness.queryMap["#modules"];
  assert.equal(modulesContainer.children.length, 1);
  const guildACard = modulesContainer.children[0];
  const saveBtn = guildACard.querySelector(".save-button");
  assert.ok(saveBtn, "saveBtn should exist on the rendered card");

  // User switches to Guild B
  const guildSelect = harness.queryMap["#guild-select"];
  await guildSelect.trigger("change", { target: { value: "guild-b" } });

  // Stale save button from Guild A is clicked
  await saveBtn.trigger("click");

  // Verify that NO PUT request was sent to Guild B with Guild A's config!
  const putToB = puts.find((p) => p.url.includes("guild-b"));
  assert.equal(putToB, undefined, "Guild A module save must NEVER be sent to Guild B");
  assert.equal(puts.length, 0, "Save on stale card must be blocked completely");

  // Verify status message shows error
  const saveStatus = harness.queryMap["#save-status"];
  assert.match(saveStatus.textContent, /został zmieniony|uległ zmianie/);
});

test("In-flight save for Guild A does not mutate Guild B UI or KPIs when it resolves after guild switch", async () => {
  let resolveSavePut;
  const savePromise = new Promise((resolve) => {
    resolveSavePut = resolve;
  });

  const harness = createTestHarness(async (url, opts = {}) => {
    if (url === "/api/session") {
      return {
        ok: true,
        status: 200,
        json: async () => ({
          user: { id: "1001", username: "Tester" },
          guilds: [
            { id: "guild-a", name: "Guild A", permissions: "32" },
            { id: "guild-b", name: "Guild B", permissions: "32" },
          ],
          csrf_token: "token",
        }),
      };
    }
    if (url === "/api/guilds/guild-a/modules") {
      return {
        ok: true,
        status: 200,
        json: async () => [
          {
            manifest: { id: "mod_a", name: "Module A", category: "General", description: "A", version: "1.0" },
            enabled: true,
            configured: false,
            config: {},
          },
        ],
      };
    }
    if (opts.method === "PUT" && url.includes("guild-a")) {
      return await savePromise;
    }
    return {
      ok: true,
      status: 200,
      json: async () => ({}),
    };
  });

  await flushTicks();

  const saveBtn = harness.queryMap["#modules"].children[0].querySelector(".save-button");
  assert.ok(saveBtn, "saveBtn should exist");

  // Start saving on Guild A
  const saveAction = saveBtn.trigger("click");

  // Switch to Guild B while save is in flight
  const guildSelect = harness.queryMap["#guild-select"];
  await guildSelect.trigger("change", { target: { value: "guild-b" } });

  // Resolve save response for Guild A
  resolveSavePut({
    ok: true,
    status: 200,
    json: async () => ({ updated_at: "2026-11-02T20:00:00Z" }),
  });
  await saveAction;
  await flushTicks();

  // Verify save status does NOT say "Zapisano pomyślnie" on Guild B
  const saveStatus = harness.queryMap["#save-status"];
  assert.notEqual(saveStatus.textContent, "Zapisano pomyślnie konfigurację modułu.");
});

test("Switching guilds resets modulesData and KPIs immediately to avoid leakage", async () => {
  const harness = createTestHarness(async (url) => {
    if (url === "/api/session") {
      return {
        ok: true,
        status: 200,
        json: async () => ({
          user: { id: "1001", username: "Tester" },
          guilds: [
            { id: "guild-a", name: "Guild A", permissions: "32" },
            { id: "guild-b", name: "Guild B", permissions: "32" },
          ],
          csrf_token: "token",
        }),
      };
    }
    if (url === "/api/guilds/guild-a/modules") {
      return {
        ok: true,
        status: 200,
        json: async () => [
          { manifest: { id: "m1" }, enabled: true },
          { manifest: { id: "m2" }, enabled: true },
        ],
      };
    }
    if (url.includes("/cases") || url.includes("/audit")) {
      return { ok: true, status: 200, json: async () => ({ total: 5, items: [] }) };
    }
    return { ok: true, status: 200, json: async () => ({}) };
  });

  await flushTicks();

  assert.equal(harness.queryMap["#kpi-active-modules"].textContent, "2 / 2");

  // Switch to Guild B
  const guildSelect = harness.queryMap["#guild-select"];
  await guildSelect.trigger("change", { target: { value: "guild-b" } });

  // modulesData must be empty immediately, KPI must be reset, not displaying 2 / 2
  assert.equal(harness.sandbox.__app.state.modulesData.length, 0);
  assert.equal(harness.queryMap["#kpi-active-modules"].textContent, "-");
});

test("Moderation case details and notes are bound to targetGuild and blocked after switching guilds", async () => {
  const posts = [];

  const harness = createTestHarness(async (url, opts = {}) => {
    if (url === "/api/session") {
      return {
        ok: true,
        status: 200,
        json: async () => ({
          user: { id: "1001", username: "Tester" },
          guilds: [
            { id: "guild-a", name: "Guild A", permissions: "32" },
            { id: "guild-b", name: "Guild B", permissions: "32" },
          ],
          csrf_token: "token",
        }),
      };
    }
    if (url.includes("/api/guilds/guild-a/cases?")) {
      return {
        ok: true,
        status: 200,
        json: async () => ({
          total: 1,
          items: [
            {
              case_number: 42,
              kind: "warn",
              status: "active",
              reason: "Spam",
              subject_user_id: "2001",
              actor_user_id: "1001",
              points: 10,
              severity: "low",
              created_at: "2026-11-02T19:00:00Z",
              version: 1,
            },
          ],
        }),
      };
    }
    if (url === "/api/guilds/guild-a/cases/42") {
      return {
        ok: true,
        status: 200,
        json: async () => ({
          case: {
            case_number: 42,
            kind: "warn",
            status: "active",
            reason: "Spam",
            points: 10,
            version: 1,
          },
          notes: [],
        }),
      };
    }
    if (opts.method === "POST" && url.includes("/notes")) {
      posts.push({ url, body: opts.body });
      return { ok: true, status: 200, json: async () => ({}) };
    }
    return { ok: true, status: 200, json: async () => ({}) };
  });

  await flushTicks();

  // Switch to moderation tab on Guild A
  const navMod = harness.queryMap["#nav-moderation"];
  await navMod.trigger("click");
  await flushTicks();

  const casesList = harness.queryMap["#cases-list"];
  assert.equal(casesList.children.length, 1);
  const card = casesList.children[0];

  // Expand case details
  const toggleBtn = card.querySelector(".toggle-details-btn");
  await toggleBtn.trigger("click");
  await flushTicks();

  const detailsContainer = card.querySelector(".case-details-container");
  const noteForm = detailsContainer.querySelector(".case-add-note-form");
  assert.ok(noteForm, "Case add note form should be rendered");

  const noteInput = noteForm.querySelector(".case-note-input");
  noteInput.value = "New note for case 42";

  // Switch to Guild B
  const guildSelect = harness.queryMap["#guild-select"];
  await guildSelect.trigger("change", { target: { value: "guild-b" } });

  // Try to submit note from Guild A case
  await noteForm.trigger("submit");

  // Verify no note POST was sent to Guild B
  const postToB = posts.find((p) => p.url.includes("guild-b"));
  assert.equal(postToB, undefined, "Note for Guild A case must never be sent to Guild B");
  assert.equal(posts.length, 0, "Note submission on stale case must be blocked");

  const saveStatus = harness.queryMap["#save-status"];
  assert.match(saveStatus.textContent, /został zmieniony|uległ zmianie/);
});

test("Voiding a case is blocked if guild changed before dialog submission", async () => {
  const posts = [];

  const harness = createTestHarness(async (url, opts = {}) => {
    if (url === "/api/session") {
      return {
        ok: true,
        status: 200,
        json: async () => ({
          user: { id: "1001", username: "Tester" },
          guilds: [
            { id: "guild-a", name: "Guild A", permissions: "32" },
            { id: "guild-b", name: "Guild B", permissions: "32" },
          ],
          csrf_token: "token",
        }),
      };
    }
    if (url.includes("/api/guilds/guild-a/cases?")) {
      return {
        ok: true,
        status: 200,
        json: async () => ({
          total: 1,
          items: [
            {
              case_number: 99,
              kind: "ban",
              status: "active",
              reason: "Raid",
              subject_user_id: "2001",
              actor_user_id: "1001",
              points: 100,
              severity: "critical",
              created_at: "2026-11-02T19:00:00Z",
              version: 1,
            },
          ],
        }),
      };
    }
    if (url === "/api/guilds/guild-a/cases/99") {
      return {
        ok: true,
        status: 200,
        json: async () => ({
          case: {
            case_number: 99,
            kind: "ban",
            status: "active",
            reason: "Raid",
            points: 100,
            version: 1,
          },
          notes: [],
        }),
      };
    }
    if (opts.method === "POST" && url.includes("/void")) {
      posts.push({ url, body: opts.body });
      return { ok: true, status: 200, json: async () => ({}) };
    }
    return { ok: true, status: 200, json: async () => ({}) };
  });

  await flushTicks();

  // Switch to moderation tab on Guild A
  const navMod = harness.queryMap["#nav-moderation"];
  await navMod.trigger("click");
  await flushTicks();

  const card = harness.queryMap["#cases-list"].children[0];
  const toggleBtn = card.querySelector(".toggle-details-btn");
  await toggleBtn.trigger("click");
  await flushTicks();

  const detailsContainer = card.querySelector(".case-details-container");
  const voidBtn = detailsContainer.querySelector(".btn-danger");
  assert.ok(voidBtn, "Void button should exist");

  // Click void to open modal
  await voidBtn.trigger("click");
  assert.ok(harness.sandbox.__app.state.pendingVoidCase, "pendingVoidCase should be set");
  assert.equal(harness.sandbox.__app.state.pendingVoidCase.targetGuild, "guild-a");

  // Guild changes before submitting
  const guildSelect = harness.queryMap["#guild-select"];
  await guildSelect.trigger("change", { target: { value: "guild-b" } });

  // Dialog was closed and pendingVoidCase cleared
  assert.equal(harness.sandbox.__app.state.pendingVoidCase, null);

  // If submit is triggered, nothing is posted
  const voidForm = harness.queryMap["#void-form"];
  await voidForm.trigger("submit");
  assert.equal(posts.length, 0, "Voiding must be blocked after guild change");
});
