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
    const renderer = dbg
      ? String(gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL))
      : ''
    const software = /swiftshader|llvmpipe|software|basic render|microsoft basic/i.test(
      renderer,
    )
    const maxTex = gl.getParameter(gl.MAX_TEXTURE_SIZE) as number
    return !software && maxTex >= 4096
  } catch {
    return false
  }
}
