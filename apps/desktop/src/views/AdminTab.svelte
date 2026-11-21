<!--
SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

Admin (`spec/05-desktop-agent.md`, Admin section): join requests, devices,
enrolment codes, and test alarms with their reports.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { api, clock } from "../lib/api";
  import StatusIcon from "../lib/StatusIcon.svelte";
  import type { Device, DrillSummary, PracticeSettings, Token } from "../lib/types";

  let {
    joins = $bindable([]),
    onerror,
  }: {
    joins?: { device: string; label: string; code: string }[];
    onerror: (msg: string) => void;
  } = $props();

  let devices = $state<Device[]>([]);
  let tokens = $state<Token[]>([]);
  let reports = $state<DrillSummary[]>([]);
  let filter = $state<"all" | "attention">("all");
  let removing = $state<string | null>(null);
  let promoting = $state<string | null>(null);
  let editing = $state<string | null>(null);
  let editLabel = $state("");
  let editRoom = $state("");
  let revokingToken = $state<string | null>(null);
  let tokenDays = $state(7);
  let tokenLabel = $state("");
  let token = $state<{ text: string; expires_at: number } | null>(null);
  let copied = $state("");
  let drillSent = $state(false);
  let settings = $state<PracticeSettings | null>(null);
  let sHold = $state("1500");
  let sCue = $state("soft");
  let sPattern = $state("A");
  let sDrill = $state("silent");
  let sHours = $state("");
  let sRelays = $state("");
  let sDay = $state("off");
  let sTime = $state("07:45");
  let sCoord = $state("");
  let settingsSaved = $state(false);
  let settingsDirty = $state(false);
  let kitOld = $state("");
  let kitOldPass = $state("");
  let kitNew = $state("");
  let kitNewPass = $state("");
  let kitNewPass2 = $state("");
  let kitBusy = $state(false);
  let kitError = $state("");
  let kitMade = $state<string | null>(null);

  async function newKit() {
    kitError = "";
    if (kitNewPass.length < 12) {
      kitError = "The new passphrase needs at least 12 characters.";
      return;
    }
    if (kitNewPass !== kitNewPass2) {
      kitError = "The two new passphrases are different.";
      return;
    }
    kitBusy = true;
    try {
      const k = await api.newRecoveryKit(kitOld.trim(), kitOldPass, kitNew.trim(), kitNewPass);
      kitMade = k.recovery_kit;
      kitOldPass = kitNewPass = kitNewPass2 = "";
    } catch (e) {
      kitError = String(e);
    } finally {
      kitBusy = false;
    }
  }

  function loadSettings(s: PracticeSettings) {
    settings = s;
    if (settingsDirty) return;
    sHold = String(s.hold_ms ?? 1500);
    sCue = s.cue_mode ?? "soft";
    sPattern = s.sound_pattern ?? "A";
    sDrill = s.drill_mode ?? "silent";
    sHours = s.opening_hours ?? "";
    sRelays = (s.relay_urls ?? []).join("\n");
    const [d, t] = (s.drill_schedule ?? "").split(" ");
    sDay = s.drill_schedule ? d : "off";
    sTime = t || "07:45";
    sCoord = s.drill_coordinator ?? devices.find((x) => x.this_device)?.device ?? "";
  }

  async function saveSettings() {
    try {
      await api.setPracticeSettings({
        hold_ms: Number(sHold),
        cue_mode: sCue,
        sound_pattern: sPattern,
        drill_mode: sDrill,
        opening_hours: sHours.trim() || null,
        drill_schedule: sDay === "off" ? null : `${sDay} ${sTime}`,
        drill_coordinator: sDay === "off" ? null : sCoord || null,
        relay_urls: sRelays
          .split("\n")
          .map((x) => x.trim())
          .filter(Boolean),
      });
      settingsDirty = false;
      settingsSaved = true;
      setTimeout(() => (settingsSaved = false), 2500);
      refresh();
    } catch (e) {
      onerror(String(e));
    }
  }

  const DAY = 86_400_000;
  // "Needs attention": not green, or not heard recently (FR-STS-005).
  const attention = (d: Device) =>
    !d.this_device && (d.status !== "green" || d.last_heard_ms === null || d.last_heard_ms > DAY);
  const shown = $derived(filter === "all" ? devices : devices.filter(attention));
  const tokenOf = (id: string | null) => tokens.find((t) => t.token_id === id)?.label;

  async function refresh() {
    try {
      let s: PracticeSettings;
      [devices, tokens, reports, s] = await Promise.all([
        api.devices(),
        api.tokens(),
        api.drillReports(),
        api.practiceSettings(),
      ]);
      loadSettings(s);
      // The agent keeps join requests while the device repeats them, so a
      // restarted app still shows them.
      joins = await api.joinRequests();
      devices.sort(
        (a, b) => Number(b.this_device) - Number(a.this_device) || (a.room ?? "").localeCompare(b.room ?? ""),
      );
    } catch (e) {
      onerror(String(e));
    }
  }

  function heard(d: Device): string {
    if (d.this_device) return "This computer";
    if (d.last_heard_ms === null) return "Not heard since this computer started";
    if (d.last_heard_ms < 60_000) return "Just now";
    return `${Math.round(d.last_heard_ms / 60_000)} min ago`;
  }

  async function remove(d: Device) {
    try {
      await api.revoke(d.device, "Removed by a Practice Admin");
      removing = null;
      refresh();
    } catch (e) {
      onerror(String(e));
    }
  }

  async function promote(d: Device) {
    try {
      await api.promote(d.device);
      promoting = null;
      refresh();
    } catch (e) {
      onerror(String(e));
    }
  }

  function startEdit(d: Device) {
    editing = d.device;
    editLabel = d.label;
    editRoom = d.room ?? "";
  }

  async function saveEdit(d: Device) {
    try {
      const label = editLabel.trim() !== d.label ? editLabel.trim() : null;
      const room = editRoom.trim() !== (d.room ?? "") ? editRoom.trim() : null;
      if (label !== null || room !== null) await api.assignSettings(d.device, label, room);
      editing = null;
      setTimeout(refresh, 1500);
    } catch (e) {
      onerror(String(e));
    }
  }

  async function makeToken() {
    try {
      token = await api.createToken(tokenDays, tokenLabel || "Enrolment code");
      copied = "";
      refresh();
    } catch (e) {
      onerror(String(e));
    }
  }

  async function revokeToken(t: Token) {
    try {
      await api.revokeToken(t.token_id);
      revokingToken = null;
      refresh();
    } catch (e) {
      onerror(String(e));
    }
  }

  async function copy(text: string, what: string) {
    await navigator.clipboard.writeText(text);
    copied = what;
    setTimeout(() => (copied = ""), 2500);
  }

  async function copyReport(r: DrillSummary) {
    try {
      const full = await api.drillReport(r.alarm_id);
      await copy(full.csv, r.alarm_id);
    } catch (e) {
      onerror(String(e));
    }
  }

  async function copyAudit() {
    try {
      const a = await api.auditExport();
      await copy(a.csv, "audit");
    } catch (e) {
      onerror(String(e));
    }
  }

  async function drill() {
    await api.runDrill();
    drillSent = true;
    setTimeout(() => (drillSent = false), 4000);
  }

  onMount(() => {
    refresh();
    api
      .appInfo()
      .then((i) => (kitNew ||= `${i.home_dir}/Panic Button recovery kit (new).panic-button-kit`))
      .catch(() => {});
    const t = setInterval(refresh, 10_000);
    const off = api.onEvent((e) => {
      if (e.type === "drill_report") refresh();
    });
    return () => {
      clearInterval(t);
      off.then((f) => f());
    };
  });
</script>

<h1>Admin</h1>

{#if joins.length}
  <section>
    <h2>Waiting to join</h2>
    {#each joins as j (j.device)}
      <div class="join">
        <div>
          <strong>{j.label}</strong><br /><span class="muted">Check this code matches their screen:</span>
          <code class="code">{j.code.slice(0, 3)}&nbsp;{j.code.slice(3)}</code>
        </div>
        <button
          class="btn primary"
          onclick={async () => {
            await api.approveJoin(j.device);
            joins = joins.filter((x) => x.device !== j.device);
          }}>Approve</button
        >
        <button
          class="btn"
          onclick={async () => {
            await api.rejectJoin(j.device);
            joins = joins.filter((x) => x.device !== j.device);
          }}>Reject</button
        >
      </div>
    {/each}
  </section>
{/if}

{#if devices.length && devices.filter((d) => d.role === "admin").length < 2}
  <p class="notice" role="status">
    This practice has only one Practice Admin. If that computer is lost, admin access needs the recovery kit. Make a
    second computer an admin below.
  </p>
{/if}

<section>
  <div class="head">
    <h2>Devices ({devices.length})</h2>
    <div class="seg" role="group" aria-label="Show">
      <button class:on={filter === "all"} aria-pressed={filter === "all"} onclick={() => (filter = "all")}>All</button>
      <button
        class:on={filter === "attention"}
        aria-pressed={filter === "attention"}
        onclick={() => (filter = "attention")}
      >
        Needs attention ({devices.filter(attention).length})
      </button>
    </div>
  </div>
  <table>
    <thead
      ><tr
        ><th>Status</th><th>Computer</th><th>Room</th><th>Person</th><th>Last heard</th><th
          ><span class="visually-hidden">Actions</span></th
        ></tr
      ></thead
    >
    <tbody>
      {#each shown as d (d.device)}
        <tr>
          <td><StatusIcon level={d.status} size={12} /></td>
          {#if editing === d.device}
            <td><input class="cell" aria-label="Computer name" bind:value={editLabel} maxlength="64" /></td>
            <td><input class="cell" aria-label="Room" bind:value={editRoom} maxlength="64" /></td>
          {:else}
            <td>
              {d.label || d.device_short}
              {#if d.role === "admin"}<span class="tag">Admin</span>{/if}
              {#if d.pending_label}<br /><span class="muted small">Changing to "{d.pending_label}"</span>{/if}
              {#if d.token_id && tokenOf(d.token_id)}<br /><span class="muted small"
                  >Joined with "{tokenOf(d.token_id)}"</span
                >{/if}
            </td>
            <td>
              {d.room || "-"}
              {#if d.pending_room}<br /><span class="muted small">Changing to "{d.pending_room}"</span>{/if}
            </td>
          {/if}
          <td>{d.user || "-"}</td>
          <td>{heard(d)}</td>
          <td class="right">
            {#if editing === d.device}
              <button class="btn primary" onclick={() => saveEdit(d)}>Save</button>
              <button class="btn" onclick={() => (editing = null)}>Cancel</button>
            {:else if d.this_device}
              <button class="btn" onclick={() => startEdit(d)}>Edit</button>
            {:else if removing === d.device}
              <span>Remove from the practice?</span>
              <button class="btn danger" onclick={() => remove(d)}>Remove</button>
              <button class="btn" onclick={() => (removing = null)}>Keep</button>
            {:else if promoting === d.device}
              <span>Let this computer manage the practice?</span>
              <button class="btn primary" onclick={() => promote(d)}>Make admin</button>
              <button class="btn" onclick={() => (promoting = null)}>Back</button>
            {:else}
              <button class="btn" onclick={() => startEdit(d)}>Edit</button>
              {#if d.role !== "admin"}
                <button class="btn" onclick={() => (promoting = d.device)}>Make admin...</button>
              {/if}
              <button class="btn" onclick={() => (removing = d.device)}>Remove...</button>
            {/if}
          </td>
        </tr>
      {:else}
        <tr><td colspan="6" class="muted">Nothing needs attention.</td></tr>
      {/each}
    </tbody>
  </table>
</section>

<section>
  <h2>Add a computer</h2>
  <p>Create an enrolment code and paste it into Panic Button on the new computer.</p>
  <div class="inline">
    <div class="field">
      <label for="tlabel">Label</label>
      <input id="tlabel" bind:value={tokenLabel} placeholder="Reception PCs" />
    </div>
    <div class="field">
      <label for="tdays">Valid for (days)</label>
      <input id="tdays" type="number" min="1" max="365" bind:value={tokenDays} />
    </div>
    <button class="btn primary" onclick={makeToken}>Create code</button>
  </div>
  {#if token}
    <div class="token">
      <textarea readonly rows="3" aria-label="Enrolment code">{token.text}</textarea>
      <div class="inline">
        <button class="btn" onclick={() => token && copy(token.text, "token")}
          >{copied === "token" ? "Copied" : "Copy"}</button
        >
        <span class="muted"
          >Valid until {new Date(token.expires_at).toLocaleDateString("en-GB")}. Treat it like a password.</span
        >
      </div>
    </div>
  {/if}
  {#if tokens.length}
    <h3>Enrolment codes</h3>
    <table>
      <thead
        ><tr
          ><th>Label</th><th>State</th><th>Computers that joined with it</th><th
            ><span class="visually-hidden">Actions</span></th
          ></tr
        ></thead
      >
      <tbody>
        {#each tokens as t (t.token_id)}
          <tr>
            <td>{t.label}</td>
            <td>
              {t.revoked
                ? "Revoked"
                : t.expired
                  ? "Expired"
                  : `Valid until ${new Date(t.expires_at).toLocaleDateString("en-GB")}`}
            </td>
            <td>{t.enrolled.map((e) => e.label).join(", ") || "-"}</td>
            <td class="right">
              {#if t.revoked || t.expired}
                <span></span>
              {:else if revokingToken === t.token_id}
                <span>Stop this code working?</span>
                <button class="btn danger" onclick={() => revokeToken(t)}>Revoke</button>
                <button class="btn" onclick={() => (revokingToken = null)}>Keep</button>
              {:else}
                <button class="btn" onclick={() => (revokingToken = t.token_id)}>Revoke...</button>
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
    <p class="muted small">
      Revoking a code stops new computers joining with it. Computers that already joined stay; remove any you do not
      recognise above.
    </p>
  {/if}
</section>

<section>
  <h2>Practice settings</h2>
  <p class="muted small">These apply to every computer in the practice, unless IT has set them on a computer.</p>
  <div class="grid" oninput={() => (settingsDirty = true)} onchange={() => (settingsDirty = true)}>
    <div class="field">
      <label for="s-hold">Hold time</label>
      <select id="s-hold" bind:value={sHold}>
        <option value="1000">1 second</option>
        <option value="1500">1.5 seconds</option>
        <option value="2000">2 seconds</option>
        <option value="2500">2.5 seconds</option>
        <option value="3000">3 seconds</option>
      </select>
    </div>
    <div class="field">
      <label for="s-cue">Sound while holding</label>
      <select id="s-cue" bind:value={sCue}>
        <option value="soft">Soft rising tone</option>
        <option value="silent">None</option>
        <option value="alarm_ramp">Quiet alarm, getting louder</option>
      </select>
    </div>
    <div class="field">
      <label for="s-pattern">Alarm sound</label>
      <div class="inline">
        <select id="s-pattern" bind:value={sPattern}>
          <option value="A">Pattern A (two tones)</option>
          <option value="B">Pattern B (three beeps)</option>
          <option value="C">Pattern C (slow call)</option>
        </select>
        <button class="btn" onclick={() => api.playTestSound(sPattern)}>Play</button>
      </div>
      <span class="hint">Check it does not sound like your fire alarm.</span>
    </div>
    <div class="field">
      <label for="s-drill">Test alarms</label>
      <select id="s-drill" bind:value={sDrill}>
        <option value="silent">Silent (a message only)</option>
        <option value="audible">With a short chime</option>
      </select>
    </div>
    <div class="field">
      <label for="s-day">Weekly test alarm</label>
      <div class="inline">
        <select id="s-day" bind:value={sDay}>
          <option value="off">Off</option>
          <option value="Mon">Monday</option>
          <option value="Tue">Tuesday</option>
          <option value="Wed">Wednesday</option>
          <option value="Thu">Thursday</option>
          <option value="Fri">Friday</option>
          <option value="Sat">Saturday</option>
          <option value="Sun">Sunday</option>
        </select>
        <input type="time" aria-label="Time" bind:value={sTime} disabled={sDay === "off"} />
      </div>
    </div>
    <div class="field">
      <label for="s-coord">Run it from</label>
      <select id="s-coord" bind:value={sCoord} disabled={sDay === "off"}>
        {#each devices as d (d.device)}
          <option value={d.device}>{d.label || d.device_short}{d.this_device ? " (this computer)" : ""}</option>
        {/each}
      </select>
      <span class="hint">Choose a computer that is on at that time.</span>
    </div>
    <div class="field">
      <label for="s-hours">Opening hours</label>
      <input id="s-hours" bind:value={sHours} placeholder="Mon-Fri 08:00-18:30" />
    </div>
    <div class="field wide">
      <label for="s-relays">Relay addresses</label>
      <textarea
        id="s-relays"
        rows="2"
        bind:value={sRelays}
        spellcheck="false"
        placeholder="wss://relay.example.org.uk/v1/ws"></textarea>
      <span class="hint"
        >One per line. Only needed for separate networks, Wi-Fi isolation or staff working elsewhere.</span
      >
    </div>
  </div>
  <button class="btn primary" onclick={saveSettings}>Save practice settings</button>
  {#if settingsSaved}<span class="saved" role="status">Saved and sent to every computer</span>{/if}
  {#if settings?.version}<span class="muted small"> Version {settings.version}</span>{/if}
</section>

<section>
  <h2>Test alarms</h2>
  <p>
    Every device shows "Test alarm received - no action needed". Nobody is alerted. The report is ready after a minute.
  </p>
  <button class="btn" onclick={drill}>Run a test alarm now</button>
  {#if drillSent}<span class="saved" role="status">Sent</span>{/if}
  {#if reports.length}
    <table>
      <thead><tr><th>When</th><th>Reached</th><th><span class="visually-hidden">Actions</span></th></tr></thead>
      <tbody>
        {#each reports as r (r.alarm_id)}
          <tr>
            <td>{new Date(r.started_at).toLocaleDateString("en-GB")} {clock(r.started_at)}</td>
            {#if r.skipped}
              <td><span class="warn">Did not run: the computer that runs it was off</span></td>
              <td></td>
            {:else}
              <td>
                {r.received} of {r.expected} computers
                {#if r.received < r.expected}<span class="warn">({r.expected - r.received} missed)</span>{/if}
              </td>
              <td class="right">
                <button class="btn" onclick={() => copyReport(r)}
                  >{copied === r.alarm_id ? "Copied" : "Copy as CSV"}</button
                >
              </td>
            {/if}
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</section>

<section>
  <h2>Audit log</h2>
  <p>This computer's record of alarms, test alarms, enrolments, removals and settings changes.</p>
  <button class="btn" onclick={copyAudit}>{copied === "audit" ? "Copied" : "Copy as CSV"}</button>
</section>

<section>
  <h2>Recovery kit</h2>
  <p>
    Make a new kit with a new passphrase, for example if someone who knew the passphrase has left. You need the current
    kit and its passphrase. The old kit keeps working, so destroy it and any copies once the new one is stored safely.
  </p>
  {#if kitMade}
    <p class="saved" role="status">New kit saved at <code>{kitMade}</code>. It opens with the new passphrase.</p>
    <button class="btn" onclick={() => (kitMade = null)}>Make another</button>
  {:else}
    <div class="field">
      <label for="kit-old">Current recovery kit file</label>
      <input id="kit-old" bind:value={kitOld} spellcheck="false" />
    </div>
    <div class="field narrow">
      <label for="kit-old-pass">Current passphrase</label>
      <input id="kit-old-pass" type="password" bind:value={kitOldPass} autocomplete="off" />
    </div>
    <div class="field narrow">
      <label for="kit-new-pass">New passphrase</label>
      <input id="kit-new-pass" type="password" bind:value={kitNewPass} autocomplete="new-password" />
      <span class="hint">At least 12 characters.</span>
    </div>
    <div class="field narrow">
      <label for="kit-new-pass2">New passphrase again</label>
      <input id="kit-new-pass2" type="password" bind:value={kitNewPass2} autocomplete="new-password" />
    </div>
    <div class="field">
      <label for="kit-new">Save the new kit to</label>
      <input id="kit-new" bind:value={kitNew} spellcheck="false" />
    </div>
    {#if kitError}<p class="error" role="alert">{kitError}</p>{/if}
    <button class="btn" disabled={kitBusy || !kitOld.trim() || !kitOldPass || !kitNew.trim()} onclick={newKit}>
      {kitBusy ? "Making the new kit (a few seconds)..." : "Make a new recovery kit"}
    </button>
  {/if}
</section>

<style>
  /* As the other tabs' headings (MainView). */
  h1 {
    font-size: 20px;
    margin: 0 0 8px;
  }

  section {
    background: var(--paper);
    border: 1px solid var(--line);
    border-radius: 10px;
    padding: 16px 20px;
  }

  h2 {
    font-size: 16px;
    margin: 0 0 10px;
  }

  h3 {
    font-size: 15px;
    margin: 18px 0 6px;
  }

  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }

  .seg {
    display: inline-flex;
    border: 1px solid var(--line);
    border-radius: 8px;
    overflow: hidden;
  }

  .seg button {
    border: 0;
    background: var(--paper);
    padding: 6px 12px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    color: var(--slate);
  }

  .seg button.on {
    background: var(--action-blue);
    color: var(--paper);
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 14px;
    margin-top: 8px;
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

  td.right {
    text-align: right;
    white-space: nowrap;
  }

  td.right .btn {
    min-height: 32px;
    font-size: 13px;
  }

  .btn.danger {
    background: var(--status-red);
    border-color: var(--status-red);
    color: var(--paper);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 0 16px;
    align-items: start;
  }

  .grid .inline {
    flex-wrap: nowrap;
    align-items: center;
  }

  .grid .inline select {
    flex: 1;
    min-width: 0;
  }

  .grid .wide {
    grid-column: 1 / -1;
  }

  .cell {
    width: 100%;
    min-height: 32px;
    padding: 2px 6px;
    border: 1px solid #9ca3af;
    border-radius: 6px;
  }

  .tag {
    margin-left: 6px;
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--mist);
    font-size: 12px;
    font-weight: 600;
  }

  .small {
    font-size: 13px;
  }

  .warn {
    color: #92400e;
    font-weight: 600;
  }

  .inline {
    display: flex;
    gap: 12px;
    align-items: flex-end;
    flex-wrap: wrap;
  }

  .inline .field {
    margin-bottom: 0;
  }

  .token {
    margin-top: 12px;
    display: grid;
    gap: 8px;
  }

  .token textarea {
    font-family: ui-monospace, monospace;
    font-size: 12px;
    padding: 8px;
    border: 1px solid var(--line);
    border-radius: 6px;
    resize: none;
  }

  .join {
    display: flex;
    gap: 10px;
    align-items: center;
    padding: 8px 0;
  }

  .join div {
    flex: 1;
  }

  .code {
    font-size: 18px;
    font-weight: 700;
    letter-spacing: 0.1em;
  }

  .notice {
    margin: 0;
    padding: 10px 14px;
    background: #fffbeb;
    border: 1px solid #fcd34d;
    border-radius: 8px;
  }

  .saved {
    margin-left: 10px;
    color: var(--status-green);
    font-weight: 600;
  }
</style>
