<script>
  // Minimal in-app modal — replaces the native prompt()/confirm() dialogs, which WebKitGTK renders
  // unstyled. The parent controls visibility ({#if ...}); Escape or an overlay click cancels;
  // Enter submits (the body is a <form>, so the confirm button is type="submit").
  let { title, onconfirm, oncancel, confirmLabel = "OK", danger = false, width = 340, confirmDisabled = false, children } = $props();
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && oncancel?.()} />

<div
  class="overlay"
  role="presentation"
  onclick={(e) => e.target === e.currentTarget && oncancel?.()}
>
  <form
    class="box"
    style="width:{width}px"
    role="dialog"
    aria-modal="true"
    aria-label={title}
    onsubmit={(e) => {
      e.preventDefault();
      onconfirm?.();
    }}
  >
    <div class="title">{title}</div>
    {@render children?.()}
    <div class="actions">
      <button type="button" class="cancel" onclick={() => oncancel?.()}>Cancel</button>
      <button type="submit" class="confirm" class:danger disabled={confirmDisabled}>{confirmLabel}</button>
    </div>
  </form>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(8, 10, 14, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }
  .box {
    width: 340px;
    background: #1b1e25;
    border: 1px solid #3a4150;
    border-radius: 12px;
    padding: 16px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.5);
  }
  .title {
    font-weight: 600;
    font-size: 14px;
    margin-bottom: 12px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }
  .actions button {
    font: inherit;
    border: 0;
    border-radius: 7px;
    padding: 6px 14px;
    cursor: pointer;
  }
  .cancel {
    background: #363b46;
    color: #e6e8ec;
  }
  .confirm {
    background: #2b7de0;
    color: #fff;
  }
  .confirm.danger {
    background: #c9403c;
  }
  .confirm:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
