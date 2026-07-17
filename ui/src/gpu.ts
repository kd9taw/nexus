// GPU-capability probe used to decide whether the 3-D globe should default ON.
// Having a WebGL context is NOT enough: many integrated or software renderers advertise
// WebGL but run a textured, bloomed globe poorly. So we also reject known SOFTWARE
// renderers (SwiftShader / llvmpipe / Microsoft Basic Render) and require a real texture
// budget. A capable machine → 3-D by default; anything else → the universal 2-D map.
export function gpuCapableForGlobe(): boolean {
  try {
    const c = document.createElement('canvas')
    const gl = (c.getContext('webgl2') ||
      c.getContext('webgl')) as WebGLRenderingContext | null
    if (!gl) return false
    const dbg = gl.getExtension('WEBGL_debug_renderer_info')
    // Fail CLOSED when the renderer is unreadable. WebKitGTK (Linux) masks
    // WEBGL_debug_renderer_info by default, so `dbg` is null and we can't tell a real
    // GPU from Mesa's llvmpipe software renderer (which still reports a huge
    // MAX_TEXTURE_SIZE and would otherwise slip through as "capable"). Without a
    // verifiable hardware renderer, default to the cheap 2-D map — the operator can
    // still opt into the globe with the 🌐 toggle. This keeps software-GL laptops off
    // the heavy bloomed globe that made the whole app crawl.
    if (!dbg) return false
    const renderer = String(gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL))
    const software = /swiftshader|llvmpipe|software|basic render|microsoft basic/i.test(
      renderer,
    )
    const maxTex = gl.getParameter(gl.MAX_TEXTURE_SIZE) as number
    return !software && maxTex >= 4096
  } catch {
    return false
  }
}
