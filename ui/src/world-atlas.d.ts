// The world-atlas TopoJSON ships as JSON; declare it as `unknown` so tsc doesn't
// infer a giant literal type for the bundled basemap.
declare module 'world-atlas/countries-50m.json' {
  const topology: unknown
  export default topology
}
declare module 'world-atlas/countries-110m.json' {
  const topology: unknown
  export default topology
}

// us-atlas ships US states/counties as geographic (lon/lat) TopoJSON — drawable on
// the same d3-geo projections as the world basemap (no separate Albers reprojection).
declare module 'us-atlas/states-10m.json' {
  const topology: unknown
  export default topology
}

// Bundled image assets (Vite returns the served URL string).
declare module '*.webp' {
  const src: string
  export default src
}

declare module '*.geojson?url' {
  const url: string
  export default url
}
