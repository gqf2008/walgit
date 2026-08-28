name: 缺陷报告
description: 一个可复现的缺陷：现象 → 根因 → 修复 → 回归测试
title: "[bug] <一句话现象>"
labels: ["bug"]
body:
  - type: textarea
    id: repro
    attributes:
      label: 复现
      description: 最小复现步骤/用例；先复现再修
    validations:
      required: true
  - type: textarea
    id: expected
    attributes:
      label: 期望 vs 实际
    validations:
      required: true
  - type: textarea
    id: env
    attributes:
      label: 环境
      description: OS / rustc / git / 配置要点
  - type: textarea
    id: regression
    attributes:
      label: 回归测试
      description: 修复必须附"修复前失败、修复后通过"的测试
