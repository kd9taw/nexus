import { describe, it, expect } from 'vitest'
import { classifyViewport } from './useViewport'

describe('classifyViewport', () => {
  it('maps effective width to the right size class at each boundary', () => {
    expect(classifyViewport(320)).toBe('xs')
    expect(classifyViewport(767)).toBe('xs')
    expect(classifyViewport(768)).toBe('sm')
    expect(classifyViewport(1099)).toBe('sm')
    expect(classifyViewport(1100)).toBe('md')
    expect(classifyViewport(1599)).toBe('md')
    expect(classifyViewport(1600)).toBe('lg')
    expect(classifyViewport(2399)).toBe('lg')
    expect(classifyViewport(2400)).toBe('xl')
    expect(classifyViewport(3840)).toBe('xl')
  })
})
