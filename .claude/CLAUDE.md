# Development Requirements and Key Methods

## Critical Development Goals (MUST FOLLOW)

### 1. Design Document Compliance

- **来源**: User instruction: "所有功能都必须达到生产可用级别。并且你一定要保证在实现任何功能和模块的时候都需要先看一下文档，而不是自己凭感觉实现"
- **要求**:
  - All features must be production-ready
  - MUST read design documents before implementing any feature
  - Do NOT implement based on assumptions - always follow design specs
  - Design docs location: `/Volumes/workspace/rust/synctv-rs-design`

### 2. Complete Implementation - No TODOs

- **来源**: User instruction: "不要有todo，完整实现，不要偷懒"
- **要求**:
  - No TODO placeholders allowed
  - Complete implementation only
  - Don't cut corners or take shortcuts

### 3. Project Type

- **来源**: User instruction: "这个项目是完全重构的，不考虑旧版兼容性"
- **要求**:
  - This is a complete rewrite
  - No backward compatibility needed
  - Can directly modify original files (no new migrations needed)
