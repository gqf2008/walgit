name: 工作单元（批次）
description: 一批同类同机制的任务：现状 / 目标 / checklist / 验收。单个 bug 用 bug 模板。
title: "[batch] <主题>"
labels: ["batch"]
body:
  - type: textarea
    id: status
    attributes:
      label: 现状
      description: 现在是什么样，为什么不够（含测量/复现，不臆测）
    validations:
      required: true
  - type: textarea
    id: goal
    attributes:
      label: 目标
      description: 完成后是什么样；越可机检越好
    validations:
      required: true
  - type: textarea
    id: checklist
    attributes:
      label: 批次清单
      description: "- [ ] 逐项；一项一件事，同类合并"
      value: |
        - [ ]
        - [ ]
    validations:
      required: true
  - type: textarea
    id: acceptance
    attributes:
      label: 验收标准
      description: 哪条命令红/绿证明完成（"违反时哪条命令会红"）
    validations:
      required: true
  - type: textarea
    id: refs
    attributes:
      label: 关联
      description: 相关 issue / PR / 文档
