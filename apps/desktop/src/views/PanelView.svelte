<!--
SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

The panel beside the button: the sender panel after a trigger
(`spec/05-desktop-agent.md`, Sender panel), and the drill toast. Neutral
colours, no flashing. The honest delivery messages must be shown.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { api, clock } from "../lib/api";
  import type { Alarm, SenderPhase } from "../lib/types";

  let own = $state<Alarm | null>(null);
  let phase = $state<SenderPhase>("requested");
  let confirm = $state<"cancel" | "resolve" | null>(null);
  let toast = $state(false);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  let hideTimer: ReturnType<typeof setTimeout> | undefined;
  let busy = $state(false);
  let keepBtn = $state<HTMLButtonElement | null>(null);

  const closed = $derived(own !== null && own.state !== "open");

  function hideSoon(ms: number) {
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => {
      if (!toast && (!own || own.state !== "open")) {
        own = null;
        api.hideWindow("panel");
      }
    }, ms);
  }

  function track(a: Alarm) {
    if (!a.own || a.drill) return;
    if (own && a.alarm_id !== own.alarm_id && a.raised_at < own.raised_at) return;
    own = a;
    if (a.state !== "open") {
      confirm = null;
      hideSoon(6000);
    }
  }

  async function act(kind: "cancel" | "resolve") {
    if (!own) return;
    busy = true;
    try {
      if (kind === "cancel") await api.cancel(own.alarm_id);
      else await api.resolve(own.alarm_id);
    } finally {
      busy = false;
      confirm = null;
    }
  }

  function ask(kind: "cancel" | "resolve") {
    confirm = kind;
    // "Keep alarm" is the default focus (`spec/05-desktop-agent.md`).
    queueMicrotask(() => keepBtn?.focus());
  }

  onMount(() => {
    api
      .alarms()
      .then((all) => {
        const mine = all.filter((a) => a.own && !a.drill && a.state === "open");
        mine.sort((a, b) => b.raised_at - a.raised_at);
        if (mine[0]) own = mine[0];
      })
      .catch(() => {});
    const off = api.onEvent((e) => {
      if (e.type === "sender_panel") {
        track(e.alarm);
        phase = e.phase;
      } else if (e.type === "alarm_updated" || e.type === "alarm_closed") {
        track(e.alarm);
      } else if (e.type === "drill_toast") {
        // Your own alarm, once closed, gives way to the test alarm notice.
        if (own && own.state !== "open") own = null;
        toast = true;
        clearTimeout(toastTimer);
        toastTimer = setTimeout(() => {
          toast = false;
          hideSoon(0);
        }, 10_000);
      }
    });
    return () => off.then((f) => f());
  });
</script>

<main>
  {#if own}
    <section aria-live="polite">
      {#if closed}
        <h1>
          {own.state === "resolved" ? "Resolved" : own.state === "cancelled" ? "Cancelled - sent in error" : "Ended"}
        </h1>
        {#if own.state === "resolved" && own.resolved_by}
          <p class="muted">By {own.resolved_by}{own.closed_at ? ` at ${clock(own.closed_at)}` : ""}</p>
        {/if}
      {:else if confirm === "cancel"}
        <p class="q">Cancel the alarm and tell everyone it was sent in error?</p>
        <div class="row">
          <button class="btn" disabled={busy} onclick={() => act("cancel")}>Cancel alarm</button>
          <button class="btn primary" bind:this={keepBtn} onclick={() => (confirm = null)}>Keep alarm</button>
        </div>
      {:else if confirm === "resolve"}
        <p class="q">Mark as resolved so everyone can stand down?</p>
        <div class="row">
          <button class="btn primary" disabled={busy} onclick={() => act("resolve")}>Resolve</button>
          <button class="btn" bind:this={keepBtn} onclick={() => (confirm = null)}>Back</button>
        </div>
      {:else}
        <h1>
          {#if phase === "requested" && own.delivered === 0}
            <span class="spinner" aria-hidden="true"></span>Help requested
          {:else}
            Delivered to {own.delivered} {own.delivered === 1 ? "device" : "devices"}
          {/if}
        </h1>
        {#if phase === "not_yet_delivered" && own.delivered === 0}
          <p class="amber">Not yet delivered - trying all routes</p>
        {:else if phase === "could_not_reach" && own.delivered === 0}
          <p class="red">Could not reach other devices. Call for help another way.</p>
        {/if}
        {#if own.coming.length}
          <p class="coming"><strong>Coming:</strong> {own.coming.join(", ")}</p>
        {:else}
          <p class="muted">Nobody has said they are coming yet.</p>
        {/if}
        <div class="row">
          <button class="btn" onclick={() => ask("cancel")}>Cancel - sent in error</button>
          <button class="btn" onclick={() => ask("resolve")}>Resolved</button>
        </div>
      {/if}
      <button class="collapse" aria-label="Hide this panel" title="Hide" onclick={() => api.hideWindow("panel")}
        >×</button
      >
    </section>
  {:else if toast}
    <section aria-live="polite">
      <h1>Test alarm received</h1>
      <p class="muted">No action needed.</p>
    </section>
  {/if}
</main>

<style>
  main {
    height: 100vh;
    border: 1px solid var(--line);
    border-radius: 10px;
    background: var(--paper);
  }

  section {
    position: relative;
    padding: 12px 14px;
  }

  h1 {
    margin: 0 28px 4px 0;
    font-size: 17px;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  p {
    margin: 0 0 8px;
    font-size: 14px;
  }

  .q {
    font-weight: 600;
  }

  .coming {
    max-height: 3.2em;
    overflow: auto;
  }

  .amber {
    color: #92400e;
    font-weight: 600;
  }

  .red {
    color: var(--status-red);
    font-weight: 700;
  }

  .row {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }

  .row .btn {
    min-height: 34px;
    font-size: 13px;
    padding: 0 0.8em;
  }

  .collapse {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 30px;
    height: 30px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    font-size: 20px;
    line-height: 1;
    cursor: pointer;
    color: var(--slate);
  }

  .collapse:hover {
    background: var(--mist);
  }

  .spinner {
    width: 14px;
    height: 14px;
    border: 2px solid var(--line);
    border-top-color: var(--slate);
    border-radius: 50%;
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation: none;
    }
  }
</style>
