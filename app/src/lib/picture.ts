// A pack's picture: any picture, cropped to a square and shrunk to 256 px, so it stays small enough to share.
import { invoke } from "@tauri-apps/api/core";
import type { Instance } from "./api";
import { toast, toastError } from "./state.svelte";
import { syncInstances } from "./install";

export const PICTURE_TYPES = "image/png,image/jpeg,image/gif,image/webp";

export async function setPackPicture(inst: Instance, file: File | undefined) {
  if (!file) return;
  try {
    const img = await createImageBitmap(file);
    const side = Math.min(img.width, img.height);
    const c = document.createElement("canvas");
    c.width = c.height = Math.min(256, side);
    c.getContext("2d")!.drawImage(img, (img.width - side) / 2, (img.height - side) / 2, side, side, 0, 0, c.width, c.height);
    const png = c.toDataURL("image/png").split(",")[1];
    await invoke("set_instance_icon", { instanceId: inst.id, png });
    await syncInstances();
    toast(`New picture for ${inst.name}.`);
  } catch (e) { toastError(e instanceof Error ? "Obelisk couldn't read that picture." : String(e)); }
}
