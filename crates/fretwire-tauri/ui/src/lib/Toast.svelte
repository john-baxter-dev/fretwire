<script>
  // Transient notification stack (bottom-right). The parent owns the list and auto-expiry; clicking
  // a toast dismisses it early. Errors land here so they can't sit stale in the status bar.
  let { toasts = [], ondismiss } = $props();
</script>

{#if toasts.length}
  <div class="stack">
    {#each toasts as t (t.id)}
      <button class="toast {t.kind}" onclick={() => ondismiss?.(t.id)} title="Dismiss">
        {t.msg}
      </button>
    {/each}
  </div>
{/if}

<style>
  .stack {
    position: fixed;
    right: 16px;
    bottom: 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    z-index: 200;
    max-width: 380px;
  }
  .toast {
    font: inherit;
    font-size: 13px;
    text-align: left;
    color: #e6e8ec;
    background: #232833;
    border: 1px solid #3a4150;
    border-left: 3px solid #3f8ae0;
    border-radius: 8px;
    padding: 10px 14px;
    cursor: pointer;
    box-shadow: 0 6px 24px rgba(0, 0, 0, 0.4);
    word-break: break-word;
  }
  .toast.error {
    border-left-color: #d9534f;
  }
  .toast.info {
    border-left-color: #3f8ae0;
  }
</style>
