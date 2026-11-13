// Where the viewer looks: which point of the map is in the top left corner
// of the screen, and how many screen pixels one map pixel takes.

export interface Camera {
  x: number
  y: number
  scale: number
}

export interface Size {
  width: number
  height: number
}

export interface Rect extends Size {
  x: number
  y: number
}

export const MIN_SCALE = 0.002
export const MAX_SCALE = 2.5

function clamp(scale: number): number {
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, scale))
}

/** The part of the map that is on screen. */
export function viewOf(camera: Camera, screen: Size): Rect {
  return {
    x: camera.x,
    y: camera.y,
    width: screen.width / camera.scale,
    height: screen.height / camera.scale,
  }
}

/** A camera that shows all of a rectangle, centred, no closer than `most`. */
export function fitRect(rect: Rect, screen: Size, margin = 24, most = 1): Camera {
  const scale = clamp(
    Math.min(
      most,
      (screen.width - 2 * margin) / Math.max(1, rect.width),
      (screen.height - 2 * margin) / Math.max(1, rect.height),
    ),
  )
  return {
    scale,
    x: rect.x - (screen.width / scale - rect.width) / 2,
    y: rect.y - (screen.height / scale - rect.height) / 2,
  }
}

/** Zooms by a factor, keeping the map point under a screen point where it is. */
export function zoomAt(camera: Camera, factor: number, at: { x: number; y: number }): Camera {
  const scale = clamp(camera.scale * factor)
  const worldX = camera.x + at.x / camera.scale
  const worldY = camera.y + at.y / camera.scale
  return { scale, x: worldX - at.x / scale, y: worldY - at.y / scale }
}

/** Moves the view by a distance on screen. */
export function panBy(camera: Camera, dx: number, dy: number): Camera {
  return { ...camera, x: camera.x + dx / camera.scale, y: camera.y + dy / camera.scale }
}

/** Where a map point is on screen. */
export function toScreen(camera: Camera, x: number, y: number): { x: number; y: number } {
  return { x: (x - camera.x) * camera.scale, y: (y - camera.y) * camera.scale }
}

/**
 * The camera for a map in which a box has changed place and size (another
 * level of the same code), such that the box stays where it is on screen
 * and as wide as it was.
 */
export function follow(camera: Camera, before: Rect, after: Rect, limits: { least: number; most: number }): Camera {
  const corner = toScreen(camera, before.x, before.y)
  const width = before.width * camera.scale
  const wanted = clamp(width / Math.max(1, after.width))
  const scale = Math.min(limits.most, Math.max(limits.least, wanted))
  if (scale === wanted) {
    // The box's corner, and so its title, stays where it was.
    return { scale, x: after.x - corner.x / scale, y: after.y - corner.y / scale }
  }
  // The limits changed its width: keep its centre in place instead.
  const centreX = corner.x + width / 2
  const centreY = corner.y + (before.height * camera.scale) / 2
  return {
    scale,
    x: after.x + after.width / 2 - centreX / scale,
    y: after.y + after.height / 2 - centreY / scale,
  }
}

/** True when a camera still shows what was worked out for another one. */
export function covers(drawnFor: { view: Rect; scale: number }, camera: Camera, screen: Size): boolean {
  const view = viewOf(camera, screen)
  const ratio = camera.scale / drawnFor.scale
  return (
    ratio > 0.8 &&
    ratio < 1.25 &&
    view.x >= drawnFor.view.x &&
    view.y >= drawnFor.view.y &&
    view.x + view.width <= drawnFor.view.x + drawnFor.view.width &&
    view.y + view.height <= drawnFor.view.y + drawnFor.view.height
  )
}
