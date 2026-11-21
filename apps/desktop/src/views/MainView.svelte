<!--
SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

The main window (`spec/05-desktop-agent.md`, Main window): status, history,
settings, admin and about; or first run if this computer is not set up.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { api, clock, when } from "../lib/api";
  import StatusIcon from "../lib/StatusIcon.svelte";
  import AdminTab from "./AdminTab.svelte";
  import FirstRun from "./FirstRun.svelte";
  import type { AgentConfig, Alarm, AppInfo, Peer, Status } from "../lib/types";

  type Tab = "status" | "history" | "settings" | "admin" | "about";
  let info = $state<AppInfo | null>(null);
  let firstRun = $state(false);
  let tab = $state<Tab>("status");
  let status = $state<Status | null>(null);
  let peers = $state<Peer[]>([]);
  let alarms = $state<Alarm[]>([]);
  let cfg = $state<AgentConfig | null>(null);
  let error = $state("");
  let quitAsk = $state(false);

  let room = $state("");
  // The room as the agent last reported it. The field follows it (an admin
  // may change the room) unless the person is typing a new one.
  let agentRoom = "";
  let roomSaved = $state(false);
  let kitPath = $state("");
  let kitPass = $state("");
  let recovering = $state(false);
  let recovered = $state(false);

  async function recover() {
    error = "";
    recovering = true;
    try {
      await api.recoverAdmin(kitPath.trim(), kitPass);
      kitPass = "";
      recovered = true;
      setTimeout(refresh, 1500);
    } catch (e) {
      error = String(e);
    } finally {
      recovering = false;
    }
  }
  let joins = $state<{ device: string; label: string; code: string }[]>([]);

  const tabs = $derived<[Tab, string][]>([
    ["status", "Status"],
    ["history", "History"],
    ["settings", "Settings"],
    ...(status?.admin ? ([["admin", "Admin"]] as [Tab, string][]) : []),
    ["about", "About"],
  ]);

  const pathName: Record<Peer["path"], string> = {
    lan: "Local network",
    unicast: "Direct",
    relay: "Via relay",
    indirect: "Seen through others",
    stale: "Not heard recently",
  };

  async function refresh() {
    try {
      info = await api.appInfo();
      // The agent also runs before this computer is in a practice.
      if (!info.enrolled) {
        firstRun = true;
        return;
      }
      if (!info.running) return;
      const [s, p, a, c] = await Promise.all([api.status(), api.peers(), api.alarms(), api.config()]);
      status = s;
      peers = p.filter((x) => x.member).sort((x, y) => (x.room ?? "").localeCompare(y.room ?? ""));
      alarms = a.sort((x, y) => y.raised_at - x.raised_at);
      if (!cfg || room === agentRoom) room = c.room;
      agentRoom = c.room;
      cfg = c;
    } catch (e) {
      error = String(e);
    }
  }

  async function saveRoom() {
    error = "";
    try {
      await api.setRoom(room);
      roomSaved = true;
      setTimeout(() => (roomSaved = false), 2500);
    } catch (e) {
      error = String(e);
    }
  }

  onMount(() => {
    refresh();
    const timer = setInterval(() => {
      if (!firstRun) refresh();
    }, 5000);
    const offs = [
      api.onStarted(refresh),
      api.onEvent((e) => {
        if (e.type === "status" && status) {
          status = { ...status, level: e.level, summary: e.summary, reasons: e.reasons };
        }
        if (e.type === "join_requested") {
          joins = [...joins.filter((j) => j.device !== e.device), e];
          tab = "admin";
        }
        if (e.type.startsWith("alarm_")) refresh();
      }),
      api.onNav((t) => {
        if (t === "quit") quitAsk = true;
        else tab = t as Tab;
      }),
      api.onError((m) => (error = m)),
    ];
    return () => {
      clearInterval(timer);
      offs.forEach((o) => o.then((f) => f()));
    };
  });
</script>

{#if firstRun && info}
  <FirstRun
    {info}
    ondone={() => {
      firstRun = false;
      api.hideWindow("main");
      refresh();
    }}
  />
{:else}
  <div class="shell">
    <header>
      <div class="title">
        <strong>Panic Button</strong>
        {#if status?.practice}<span class="muted">{status.practice}</span>{/if}
      </div>
      <nav aria-label="Sections">
        {#each tabs as [id, name] (id)}
          <button class:active={tab === id} aria-current={tab === id ? "page" : undefined} onclick={() => (tab = id)}>
            {name}
          </button>
        {/each}
      </nav>
    </header>

    {#if error}
      <p class="banner error" role="alert">
        {error} <button class="link" onclick={() => (error = "")}>Dismiss</button>
      </p>
    {/if}

    <main>
      {#if tab === "status"}
        <section class="status-card level-{status?.level ?? 'none'}">
          <StatusIcon level={status?.level ?? null} size={28} />
          <div>
            <h1>{status?.summary ?? "Starting..."}</h1>
            {#each status?.reasons ?? [] as r (r)}<p>{r}</p>{/each}
            {#if info?.wayland}
              <p>
                On-screen button may be hidden behind windows on this desktop. Use the tray icon to reach Panic Button.
              </p>
            {/if}
          </div>
        </section>
        <section>
          <h2>Devices in your practice</h2>
          {#if peers.length}
            <table>
              <thead><tr><th>Room</th><th>Person</th><th>Computer</th><th>How it is reached</th></tr></thead>
              <tbody>
                {#each peers as p (p.device)}
                  <tr>
                    <td>{p.room || "-"}</td>
                    <td>{p.user || "-"}</td>
                    <td>{p.label || p.device_short}</td>
                    <td>{pathName[p.path]}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {:else}
            <p class="muted">No other devices found yet.</p>
          {/if}
        </section>
        <section>
          <h2>Sound</h2>
          {#if info && !info.audio_ok}
            <p class="error">No sound output found. Alarms on this computer will be silent.</p>
          {/if}
          <button class="btn" onclick={() => api.playTestSound(cfg?.sound_pattern)}>Play test sound</button>
        </section>
      {:else if tab === "history"}
        <section>
          <h1>History</h1>
          {#if alarms.length}
            <table>
              <thead><tr><th>When</th><th>Room</th><th>From</th><th>Outcome</th><th>Coming</th></tr></thead>
              <tbody>
                {#each alarms as a (a.alarm_id)}
                  <tr>
                    <td>{when(a.raised_at)}</td>
                    <td>{a.room || "-"}{a.drill ? " (test)" : ""}</td>
                    <td>{a.own ? "This computer" : a.user}</td>
                    <td>
                      {a.state === "open"
                        ? "Open"
                        : a.state === "resolved"
                          ? `Resolved by ${a.resolved_by ?? "someone"}${a.closed_at ? ` at ${clock(a.closed_at)}` : ""}`
                          : a.state === "cancelled"
                            ? "Cancelled - sent in error"
                            : "Expired"}
                    </td>
                    <td>{a.coming.join(", ") || "-"}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {:else}
            <p class="muted">No alarms yet.</p>
          {/if}
        </section>
      {:else if tab === "settings"}
        <section>
          <h1>Settings</h1>
          <div class="field narrow">
            <label for="room">Room</label>
            <input id="room" bind:value={room} disabled={cfg?.room_locked} />
            <span class="hint">
              {cfg?.room_locked
                ? "Set by this computer's IT configuration."
                : "Shown to colleagues when you call for help."}
            </span>
          </div>
          <button class="btn primary" onclick={saveRoom} disabled={cfg?.room_locked}>Save room</button>
          {#if roomSaved}<span class="saved" role="status">Saved</span>{/if}
          <h2>Alarm</h2>
          <dl>
            <dt>Hold time</dt>
            <dd>{cfg ? (cfg.hold_ms / 1000).toFixed(1) : "-"} seconds</dd>
            <dt>Sound</dt>
            <dd>Pattern {cfg?.sound_pattern ?? "A"}</dd>
          </dl>
          <p class="muted">Your Practice Admin sets these for the whole practice.</p>
          <h2>Keyboard shortcut</h2>
          {#if info?.shortcut?.active}
            <p>
              Hold <kbd>{info.shortcut.keys}</kbd> for {cfg ? (cfg.hold_ms / 1000).toFixed(1) : "1.5"} seconds to call for
              help, from any program. It works like holding the red button.
            </p>
          {:else if info?.shortcut}
            <p>{info.shortcut.problem}</p>
          {/if}
          <details class="recover">
            <summary>Recover admin access</summary>
            <p>
              If every Practice Admin computer is lost, the recovery kit and its passphrase make this computer an admin.
            </p>
            <div class="field narrow">
              <label for="kit">Recovery kit file</label>
              <input
                id="kit"
                bind:value={kitPath}
                spellcheck="false"
                placeholder="/media/usb/Panic Button recovery kit.panic-button-kit"
              />
            </div>
            <div class="field narrow">
              <label for="kitpass">Passphrase</label>
              <input id="kitpass" type="password" bind:value={kitPass} autocomplete="off" />
            </div>
            <button class="btn" disabled={recovering || !kitPath.trim() || !kitPass} onclick={recover}>
              {recovering ? "Opening the kit (a few seconds)..." : "Make this computer an admin"}
            </button>
            {#if recovered}<span class="saved" role="status">Done. This computer is now a Practice Admin.</span>{/if}
          </details>
          <p class="muted">
            To use a different sound on this computer, put a WAV file named <code>alarm-a.wav</code> (or
            <code>alarm-b.wav</code>, <code>alarm-c.wav</code>) in <code>{info?.sounds_dir}</code>.
          </p>
          {#if info?.allow_quit}
            <h2>Quit</h2>
            <p>
              Closing the window keeps Panic Button running. Quit stops it until you next sign in, or open it from your
              applications.
            </p>
            <button class="btn" onclick={() => (quitAsk = true)}>Quit Panic Button...</button>
          {/if}
        </section>
      {:else if tab === "admin" && status?.admin}
        <AdminTab bind:joins onerror={(m) => (error = m)} />
      {:else if tab === "about"}
        <section>
          <h1>About Panic Button</h1>
          <p>
            Version {info?.version}. A staff alarm for calling colleagues for help, from the Open Health Technology
            Foundation.
          </p>
          <p>
            Panic Button is free software under the GNU Affero General Public License, version 3 or later, with an app
            store exception. The source is at <code>https://github.com/ohtf/panic-button</code>.
          </p>
          <p class="muted">Data folder: <code>{info?.data_dir}</code></p>
        </section>
      {/if}
    </main>
  </div>
{/if}

{#if quitAsk}
  <div class="scrim">
    <div class="dialog" role="alertdialog" aria-labelledby="quit-h" aria-modal="true">
      <h2 id="quit-h">Quit Panic Button?</h2>
      <p>
        Until Panic Button starts again, this computer cannot call for help or receive alarms, and colleagues will see
        it as not connected.
      </p>
      <div class="inline end">
        <button class="btn primary" onclick={() => (quitAsk = false)}>Keep running</button>
        <button class="btn" onclick={() => api.quit().catch((e) => (error = String(e)))}>Quit</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .shell {
    min-height: 100vh;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 10px 20px;
    background: var(--paper);
    border-bottom: 1px solid var(--line);
  }

  .title {
    display: flex;
    gap: 10px;
    align-items: baseline;
    font-size: 17px;
  }

  nav {
    display: flex;
    gap: 4px;
  }

  nav button {
    min-height: 36px;
    padding: 0 14px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    font-weight: 600;
    color: var(--slate);
    cursor: pointer;
  }

  nav button:hover {
    background: var(--mist);
  }

  nav button.active {
    background: var(--mist);
    color: var(--ink);
    box-shadow: inset 0 -3px 0 var(--action-blue);
  }

  main {
    max-width: 920px;
    margin: 0 auto;
    padding: 20px;
    display: grid;
    gap: 16px;
  }

  section {
    background: var(--paper);
    border: 1px solid var(--line);
    border-radius: 10px;
    padding: 16px 20px;
  }

  h1 {
    font-size: 20px;
    margin: 0 0 8px;
  }

  h2 {
    font-size: 16px;
    margin: 0 0 10px;
  }

  .status-card {
    display: flex;
    gap: 16px;
    align-items: flex-start;
  }

  .status-card p {
    margin: 2px 0;
  }

  .level-amber {
    border-left: 6px solid var(--status-amber);
  }

  .level-red {
    border-left: 6px solid var(--status-red);
  }

  .level-green {
    border-left: 6px solid var(--status-green);
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 14px;
  }

  th,
  td {
    text-align: left;
    padding: 8px 6px;
    border-bottom: 1px solid var(--mist);
    vertical-align: middle;
  }

  th {
    color: var(--slate);
    font-weight: 600;
  }

  .inline {
    display: flex;
    gap: 12px;
    align-items: flex-end;
    flex-wrap: wrap;
  }

  .end {
    justify-content: flex-end;
  }

  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 6px 16px;
  }

  dd {
    margin: 0;
  }

  .recover {
    margin-top: 16px;
  }

  .recover summary {
    cursor: pointer;
    font-weight: 600;
  }

  .saved {
    margin-left: 10px;
    color: var(--status-green);
    font-weight: 600;
  }

  .banner {
    margin: 12px 20px 0;
    padding: 10px 14px;
    background: #fef2f2;
    border: 1px solid #fecaca;
    border-radius: 8px;
  }

  .scrim {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.35);
    display: grid;
    place-items: center;
  }

  .dialog {
    background: var(--paper);
    border-radius: 12px;
    padding: 20px 24px;
    max-width: 420px;
  }

  code {
    word-break: break-all;
  }

  kbd {
    padding: 1px 6px;
    border: 1px solid var(--line);
    border-bottom-width: 2px;
    border-radius: 4px;
    font: inherit;
    font-weight: 600;
    white-space: nowrap;
  }
</style>
