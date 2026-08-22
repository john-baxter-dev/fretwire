<script>
  // A model's icon: the pedal/amp/cab it is modelled on, as a filled silhouette. Pass the block's
  // `symbolic_id` and `category` (and `name`, which is where a cab's driver array lives); with no
  // symbolic id it falls back to the category icon, so the picker's category list uses it too.
  import { iconSpec } from "./spec.js";
  import { iconBody } from "./render.js";

  let { symbolicId = null, category = null, name = "", size = 24, dim = false } = $props();

  const markup = $derived(iconBody(iconSpec(symbolicId, category, name)));
</script>

<svg
  class="icon"
  class:dim
  viewBox="0 0 32 32"
  width={size}
  height={size}
  aria-hidden="true"
  focusable="false">{@html markup}</svg>

<style>
  .icon {
    flex: none;
    display: block;
  }
  /* Bypassed blocks: the icon fades with the label rather than disappearing. */
  .dim {
    opacity: 0.4;
  }
</style>
