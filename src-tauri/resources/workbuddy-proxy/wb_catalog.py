"""Built-in model catalog for the WorkBuddy international realm.

Snapshot of the model list the desktop app receives from
www.workbuddy.ai, shipped so that a machine without the desktop app
(and therefore without its cache) still sees the full catalog. The
CLI-facing model endpoint returns a narrower list that omits models
such as deepseek-v4.1-flash and gpt-6-astra.

Live sources take precedence: whatever the app cache or the API reports
is overlaid on top of this catalog by merge_catalog().

Stored as JSON text and parsed at import time so the literals stay
valid JSON (true/false/null) instead of needing Python spellings.
"""

import json

_JSON = r'''
[
  {
    "id": "default-model",
    "name": "Auto",
    "descriptionEn": "Excellent coding model, great for daily use",
    "descriptionZh": "优秀的编码模型，适合日常使用",
    "credits": "",
    "maxInputTokens": 176000,
    "maxOutputTokens": 24000,
    "maxAllowedSize": 200000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "relatedModels": {
      "lite": "default-model-lite",
      "reasoning": "default-model"
    },
    "temperature": 1,
    "vendor": "e",
    "isDefault": true
  },
  {
    "id": "fast-model",
    "name": "Fast",
    "descriptionEn": "Fast responses for simple tasks",
    "descriptionZh": "响应快，适合简单任务",
    "credits": "x0.34 credits",
    "maxInputTokens": 200000,
    "maxOutputTokens": 32000,
    "maxAllowedSize": 200000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "temperature": 1,
    "vendor": "i"
  },
  {
    "id": "balanced-model",
    "name": "Balanced",
    "descriptionEn": "Balanced speed and quality for daily working",
    "descriptionZh": "速度与质量兼顾，日常工作首选",
    "credits": "x0.59 credits",
    "maxInputTokens": 256000,
    "maxOutputTokens": 32000,
    "maxAllowedSize": 256000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "temperature": 1,
    "vendor": "f",
    "tags": [
      "craft"
    ]
  },
  {
    "id": "primary-model",
    "name": "Primary",
    "descriptionEn": "High-quality output for complex challenges",
    "descriptionZh": "高质量输出，胜任复杂任务",
    "credits": "x3.31 credits",
    "maxInputTokens": 272000,
    "maxOutputTokens": 72000,
    "maxAllowedSize": 272000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "reasoning": {
      "defaultEffort": "high",
      "supportedEfforts": [
        "low",
        "high",
        "max"
      ],
      "summary": "auto"
    },
    "vendor": "e"
  },
  {
    "id": "deep-model",
    "name": "Deep",
    "descriptionEn": "Deep reasoning for analysis and hard problems",
    "descriptionZh": "深度推理，适合深度分析与难题",
    "credits": "x3.33 credits",
    "maxInputTokens": 176000,
    "maxOutputTokens": 24000,
    "maxAllowedSize": 200000,
    "supportsImages": true,
    "supportsToolCall": true,
    "temperature": 1,
    "vendor": "e"
  },
  {
    "id": "deepseek-v4.1-flash",
    "name": "Deepseek-V4.1-Flash",
    "descriptionEn": "DeepSeek flagship model, supporting 1M context window, native multimodal model",
    "descriptionZh": "DeepSeek 旗舰模型，支持 1M 上下文窗口，原生多模态",
    "credits": "x0.00",
    "maxInputTokens": 1000000,
    "maxOutputTokens": 128000,
    "maxAllowedSize": 1000000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "reasoning": {
      "defaultEffort": "high",
      "supportedEfforts": [
        "low",
        "high",
        "max"
      ],
      "summary": "auto"
    },
    "contextWindow": {
      "defaultLength": 300000,
      "supportedLengths": [
        300000,
        1000000
      ]
    },
    "relatedModels": {
      "lite": "deepseek-v4.1-flash",
      "reasoning": "deepseek-v4.1-flash"
    },
    "temperature": 1,
    "vendor": "f"
  },
  {
    "id": "gpt-6-astra",
    "name": "GPT-6-Astra",
    "descriptionEn": "OpenAI's flagship model for complex reasoning and long-horizon task",
    "descriptionZh": "OpenAI 旗舰模型，擅长复杂推理与长程任务",
    "credits": "x6.67",
    "maxInputTokens": 1000000,
    "maxOutputTokens": 128000,
    "maxAllowedSize": 1000000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": true,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "low",
        "medium",
        "high",
        "xhigh",
        "max"
      ]
    },
    "contextWindow": {
      "defaultLength": 400000,
      "supportedLengths": [
        400000,
        1000000
      ]
    },
    "vendor": "e"
  },
  {
    "id": "hy4-preview-f",
    "name": "Hy4 preview",
    "descriptionEn": "Hunyuan's thinking model with enhanced reasoning capabilities",
    "descriptionZh": "混元思考模型，具有增强的推理能力",
    "credits": "x0.00",
    "maxInputTokens": 1000000,
    "maxOutputTokens": 64000,
    "maxAllowedSize": 1000000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "disabledMultimodal": false,
    "reasoning": {
      "canDisableThinking": false,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "high"
      ]
    },
    "contextWindow": {
      "defaultLength": 300000,
      "supportedLengths": [
        300000,
        1000000
      ]
    },
    "temperature": 0.9,
    "top_p": 1,
    "vendor": "j"
  },
  {
    "id": "hy4-preview",
    "name": "Hy4 preview",
    "descriptionEn": "Hunyuan's thinking model with enhanced reasoning capabilities",
    "descriptionZh": "混元思考模型，具有增强的推理能力",
    "credits": "x0.29",
    "maxInputTokens": 1000000,
    "maxOutputTokens": 64000,
    "maxAllowedSize": 1000000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "disabledMultimodal": false,
    "reasoning": {
      "canDisableThinking": false,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "high"
      ]
    },
    "contextWindow": {
      "defaultLength": 200000,
      "supportedLengths": [
        200000,
        1000000
      ]
    },
    "temperature": 0.9,
    "top_p": 1,
    "vendor": "j"
  },
  {
    "id": "hy3",
    "name": "Hy3",
    "descriptionEn": "Hunyuan's thinking model with enhanced reasoning capabilities",
    "descriptionZh": "混元思考模型，具有增强的推理能力",
    "credits": "x0.00",
    "maxInputTokens": 192000,
    "maxOutputTokens": 64000,
    "maxAllowedSize": 192000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "disabledMultimodal": false,
    "reasoning": {
      "canDisableThinking": false,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "low",
        "high"
      ]
    },
    "relatedModels": {
      "lite": "hy3",
      "reasoning": "hy3"
    },
    "temperature": 0.9,
    "top_p": 1,
    "vendor": "j",
    "tags": [
      "craft"
    ]
  },
  {
    "id": "gpt-5.6-sol",
    "name": "GPT-5.6-Sol",
    "descriptionEn": "OpenAI's flagship model for complex reasoning and long-horizon task",
    "descriptionZh": "OpenAI 旗舰模型，擅长复杂推理与长程任务",
    "credits": "x3.47",
    "maxInputTokens": 1000000,
    "maxOutputTokens": 128000,
    "maxAllowedSize": 1000000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": false,
    "reasoning": {
      "canDisableThinking": true,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "low",
        "medium",
        "high",
        "xhigh",
        "max"
      ]
    },
    "relatedModels": {
      "lite": "gpt-5.6-sol",
      "reasoning": "gpt-5.6-sol"
    },
    "vendor": "e"
  },
  {
    "id": "gpt-5.6-terra",
    "name": "GPT-5.6-Terra",
    "descriptionEn": "OpenAI's balanced model for capability, speed, and cost",
    "descriptionZh": "OpenAI 均衡模型，兼顾能力、速度与成本",
    "credits": "x1.39",
    "maxInputTokens": 1000000,
    "maxOutputTokens": 128000,
    "maxAllowedSize": 1000000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": false,
    "reasoning": {
      "canDisableThinking": true,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "low",
        "medium",
        "high",
        "xhigh",
        "max"
      ]
    },
    "relatedModels": {
      "lite": "gpt-5.6-terra",
      "reasoning": "gpt-5.6-terra"
    },
    "vendor": "e"
  },
  {
    "id": "gpt-5.6-luna",
    "name": "GPT-5.6-Luna",
    "descriptionEn": "OpenAI's lightweight model for fast responses and everyday tasks",
    "descriptionZh": "OpenAI 轻量模型，响应快速，适合日常任务",
    "credits": "x0.14",
    "maxInputTokens": 1000000,
    "maxOutputTokens": 128000,
    "maxAllowedSize": 1000000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": false,
    "reasoning": {
      "canDisableThinking": true,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "low",
        "medium",
        "high",
        "xhigh",
        "max"
      ]
    },
    "relatedModels": {
      "lite": "gpt-5.6-luna",
      "reasoning": "gpt-5.6-luna"
    },
    "vendor": "e"
  },
  {
    "id": "gpt-5.5",
    "name": "GPT-5.5",
    "descriptionEn": "OpenAI's flagship model, excelling at long-horizon tasks",
    "descriptionZh": "OpenAI 旗舰编码模型，擅长长程任务",
    "credits": "x3.31",
    "maxInputTokens": 1000000,
    "maxOutputTokens": 128000,
    "maxAllowedSize": 1000000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": false,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "low",
        "medium",
        "high",
        "xhigh"
      ]
    },
    "relatedModels": {
      "lite": "gpt-5.5",
      "reasoning": "gpt-5.5"
    },
    "vendor": "e"
  },
  {
    "id": "gpt-5.4",
    "name": "GPT-5.4",
    "descriptionEn": "OpenAI's flagship model, excelling at long-horizon tasks",
    "descriptionZh": "OpenAI 旗舰编码模型，擅长长程任务",
    "credits": "x1.65",
    "maxInputTokens": 272000,
    "maxOutputTokens": 72000,
    "maxAllowedSize": 272000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": false,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "low",
        "medium",
        "high",
        "xhigh"
      ]
    },
    "relatedModels": {
      "lite": "gpt-5.4",
      "reasoning": "gpt-5.4"
    },
    "vendor": "e",
    "tags": [
      "craft"
    ]
  },
  {
    "id": "grok-4.7",
    "name": "Grok-4.7",
    "descriptionEn": "SpaceXAI's frontier model built for coding, agentic tasks, and knowledge work.",
    "descriptionZh": "SpaceXAI 打造的前沿模型，专为编程、智能体任务及知识型工作而设计。",
    "credits": "x1.90",
    "maxInputTokens": 500000,
    "maxOutputTokens": 128000,
    "maxAllowedSize": 500000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": false,
      "defaultEffort": "high",
      "supportedEfforts": [
        "low",
        "medium",
        "high",
        "xhigh"
      ]
    },
    "relatedModels": {
      "lite": "grok-4.7",
      "reasoning": "grok-4.7"
    },
    "contextWindow": {
      "defaultLength": 200000,
      "supportedLengths": [
        200000,
        500000
      ]
    },
    "temperature": 1,
    "vendor": "f"
  },
  {
    "id": "gpt-5.3-codex",
    "name": "GPT-5.3-Codex",
    "descriptionEn": "OpenAI's coding-specialized model, great for complex coding tasks",
    "descriptionZh": "OpenAI 代码专用模型，非常擅长处理复杂的编码任务",
    "credits": "x1.25",
    "maxInputTokens": 272000,
    "maxOutputTokens": 72000,
    "maxAllowedSize": 272000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "relatedModels": {
      "lite": "default-model-lite",
      "reasoning": "gpt-5.3-codex"
    },
    "vendor": "e"
  },
  {
    "id": "gemini-3.5-flash",
    "name": "Gemini-3.5-Flash",
    "descriptionEn": "Well-rounded model for everyday use",
    "descriptionZh": "能力均衡，适合日常使用",
    "credits": "x0.99",
    "maxInputTokens": 1000000,
    "maxOutputTokens": 65536,
    "maxAllowedSize": 1000000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "relatedModels": {
      "lite": "gemini-3.5-flash",
      "reasoning": "gemini-3.5-flash"
    },
    "temperature": 1,
    "vendor": "e"
  },
  {
    "id": "glm-5.3",
    "name": "GLM-5.3",
    "descriptionEn": "Great for daily use",
    "descriptionZh": "能力均衡，适合日常使用",
    "credits": "x0.79",
    "maxInputTokens": 1000000,
    "maxOutputTokens": 48000,
    "maxAllowedSize": 1000000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": true,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "low",
        "high",
        "max"
      ]
    },
    "relatedModels": {
      "lite": "glm-5.3",
      "reasoning": "glm-5.3"
    },
    "temperature": 1,
    "vendor": "e",
    "tags": [
      "craft"
    ]
  },
  {
    "id": "glm-5.3-flash",
    "name": "GLM-5.3-Flash",
    "descriptionEn": "Native multimodal model, excelling at visual understanding and professional tasks",
    "descriptionZh": "原生多模态模型，擅长视觉理解与专业任务",
    "credits": "x0.06",
    "maxInputTokens": 1000000,
    "maxOutputTokens": 32000,
    "maxAllowedSize": 1000000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": true,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "low",
        "high",
        "max"
      ]
    },
    "relatedModels": {
      "lite": "glm-5.3-flash",
      "reasoning": "glm-5.3-flash"
    },
    "temperature": 1,
    "vendor": "f",
    "tags": [
      "craft"
    ]
  },
  {
    "id": "glm-5.2",
    "name": "GLM-5.2",
    "descriptionEn": "1M context, built for long-horizon tasks.",
    "descriptionZh": "1M 上下文，擅长长程任务",
    "credits": "x0.79",
    "maxInputTokens": 1000000,
    "maxOutputTokens": 48000,
    "maxAllowedSize": 1000000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": false,
    "reasoning": {
      "canDisableThinking": true,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "high",
        "xhigh"
      ]
    },
    "relatedModels": {
      "lite": "glm-5.2",
      "reasoning": "glm-5.2"
    },
    "temperature": 1,
    "vendor": "e",
    "tags": [
      "craft"
    ]
  },
  {
    "id": "kimi-k3",
    "name": "Kimi-K3",
    "descriptionEn": "Excels at complex, long-horizon autonomous tasks, with standout front-end skills and strong knowledge work and scientific reasoning",
    "descriptionZh": "擅长处理复杂的长程自主任务，前端开发能力突出，同时在知识工作与科研推理上表现出色。",
    "credits": "x1.62",
    "maxInputTokens": 1000000,
    "maxOutputTokens": 32000,
    "maxAllowedSize": 1000000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "relatedModels": {
      "lite": "kimi-k3-1",
      "reasoning": "kimi-k3-1"
    },
    "temperature": 1,
    "vendor": "f"
  },
  {
    "id": "kimi-k2.8-preview",
    "name": "Kimi-K2.8-Preview",
    "descriptionEn": "Excels at complex, long-horizon autonomous tasks, with standout front-end skills and strong knowledge work and scientific reasoning",
    "descriptionZh": "擅长处理复杂的长程自主任务，前端开发能力突出，同时在知识工作与科研推理上表现出色。",
    "credits": "x0.77",
    "maxInputTokens": 1000000,
    "maxOutputTokens": 32000,
    "maxAllowedSize": 1000000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": true,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "low",
        "high",
        "max"
      ]
    },
    "relatedModels": {
      "lite": "kimi-k2.8-preview",
      "reasoning": "kimi-k2.8-preview"
    },
    "temperature": 1,
    "vendor": "f",
    "tags": [
      "craft"
    ]
  },
  {
    "id": "kimi-k2.6",
    "name": "Kimi-K2.6",
    "descriptionEn": "A multimodal model, good for daily use.",
    "descriptionZh": "多模态模型，适合日常任务",
    "credits": "x0.52",
    "maxInputTokens": 256000,
    "maxOutputTokens": 32000,
    "maxAllowedSize": 256000,
    "supportsImages": true,
    "supportsToolCall": true,
    "supportsReasoning": true,
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "relatedModels": {
      "lite": "kimi-k2.6",
      "reasoning": "kimi-k2.6"
    },
    "temperature": 1,
    "vendor": "f",
    "tags": [
      "craft"
    ]
  }
]
'''

STATIC_MODELS = json.loads(_JSON)

STATIC_INTL_MODELS = STATIC_MODELS

_CN_JSON = r'''
[
  {
    "credits": "x0.00",
    "descriptionEn": "Hunyuan's thinking model with enhanced reasoning capabilities",
    "descriptionZh": "混元思考模型，具有增强的推理能力",
    "disabledMultimodal": false,
    "iconUrl": "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxNyIgaGVpZ2h0PSIxNiIgdmlld0JveD0iLTAuNDM3NSAwIDMxLjg3NSAzMCIgZmlsbD0ibm9uZSI+CiAgPHBhdGggZD0iTTE1LjIyODcgMzBDMjMuNDYwMyAzMCAzMC4xMzMzIDIzLjI4NDMgMzAuMTMzMyAxNUMzMC4xMzMzIDYuNzE1NzMgMjMuNDYwMyAwIDE1LjIyODcgMEM2Ljk5NzIgMCAwLjMyNDIxOSA2LjcxNTczIDAuMzI0MjE5IDE1QzAuMzI0MjE5IDIzLjI4NDMgNi45OTcyIDMwIDE1LjIyODcgMzBaIiBmaWxsPSIjQjNEREYyIi8+CiAgPHBhdGggZD0iTTE1LjIzMTggMjkuOTk2NkMxOS4zNDY3IDI5Ljk5NjYgMjIuNjgyNCAyNi42Mzk1IDIyLjY4MjQgMjIuNDk4M0MyMi42ODI0IDE4LjM1NzEgMTkuMzQ2NyAxNSAxNS4yMzE4IDE1QzExLjExNyAxNSA3Ljc4MTI1IDE4LjM1NzEgNy43ODEyNSAyMi40OTgzQzcuNzgxMjUgMjYuNjM5NSAxMS4xMTcgMjkuOTk2NiAxNS4yMzE4IDI5Ljk5NjZaIiBmaWxsPSIjMDA1M0UwIi8+CiAgPG1hc2sgaWQ9Im1hc2swXzYyMV8zMTE1IiBzdHlsZT0ibWFzay10eXBlOmx1bWluYW5jZSIgbWFza1VuaXRzPSJ1c2VyU3BhY2VPblVzZSIgeD0iMCIgeT0iMCIgd2lkdGg9IjMxIiBoZWlnaHQ9IjMwIj4KICAgIDxwYXRoIGQ9Ik0xNS4yMzI2IDMwQzIzLjQ2NDIgMzAgMzAuMTM3MiAyMy4yODQzIDMwLjEzNzIgMTVDMzAuMTM3MiA2LjcxNTczIDIzLjQ2NDIgMCAxNS4yMzI2IDBDNy4wMDExMSAwIDAuMzI4MTI1IDYuNzE1NzMgMC4zMjgxMjUgMTVDMC4zMjgxMjUgMjMuMjg0MyA3LjAwMTExIDMwIDE1LjIzMjYgMzBaIiBmaWxsPSJ3aGl0ZSIvPgogIDwvbWFzaz4KICA8ZyBtYXNrPSJ1cmwoI21hc2swXzYyMV8zMTE1KSI+CiAgICA8cGF0aCBkPSJNMTUuMjMyNCAzMEwzMC4xMzcgMzBMMzAuMTM3IDBMMTUuMjMyNCAwTDE1LjIzMjQgMzBaIiBmaWxsPSIjMDA1M0UwIi8+CiAgPC9nPgogIDxwYXRoIGQ9Ik0xNS4yMzE4IDE0Ljk5NjZDMTkuMzQ2NyAxNC45OTY2IDIyLjY4MjQgMTEuNjM5NSAyMi42ODI0IDcuNDk4MzFDMjIuNjgyNCAzLjM1NzExIDE5LjM0NjcgMCAxNS4yMzE4IDBDMTEuMTE3IDAgNy43ODEyNSAzLjM1NzExIDcuNzgxMjUgNy40OTgzMUM3Ljc4MTI1IDExLjYzOTUgMTEuMTE3IDE0Ljk5NjYgMTUuMjMxOCAxNC45OTY2WiIgZmlsbD0iI0IzRERGMiIvPgogIDxwYXRoIGQ9Ik0xNS4zNzEyIDAuMDA2NzRDMTguNTA2OSAwLjA4NDI5IDIxLjAyMjggMi42NjAxNSAyMS4wMjI4IDUuODMyNzdDMjEuMDIyOCA4LjUwOTc3IDE5LjIyNzEgMTAuODMyOCAxNi42MDA3IDExLjUwMzdDMTQuNDA5NyAxMS45NzU3IDEyLjcyNDYgMTMuOTg1MiAxMi43MjQ2IDE2LjM5MjRDMTIuNzI0NiAxOS4xNjA1IDE0Ljk1MjQgMjEuNDAyNiAxNy43MDI4IDIxLjQwMjZDMTguNDE2NCAyMS40MDI2IDE5LjM4NzkgMjEuMjM0IDIwLjAwMSAyMC45NjA5QzIzLjk2NzUgMTkuMzQ5MyAyNi40NyAxNS40NTUyIDI2LjQ3IDEwLjg4NjdDMjYuNDcgNC44NzE4OCAyMS42MjU4IDAgMTUuNjU1OSAwQzE1LjU1ODggMCAxNS40NjUgMC4wMDMzNyAxNS4zNzEyIDAuMDA2NzRaIiBmaWxsPSIjMkFCOUZGIi8+CiAgPHBhdGggZD0iTTQuMjAyMjIgMTguOTAyOEM0LjIwMjIyIDE1Ljg3MTggNS40MTE2IDEzLjEyNCA3LjM2ODA1IDExLjEyMTNDNy44NzcyNiAxMC41NTQ5IDguMTkyMTcgOS44MTMxMyA4LjE5MjE3IDguOTkzODRDOC4xOTIxNyA3LjI1NzUgNi43OTUxOCA1Ljg1MTU2IDUuMDY5ODkgNS44NTE1NkMzLjg3MDU2IDUuODUxNTYgMi44Mjg2OCA2LjUzNTk5IDIuMzA2MDcgNy41MzczM0MxLjA0OTc5IDkuNzM1NTggMC4zMjYxNzIgMTIuMjgxMSAwLjMyNjE3MiAxNS4wMDE5QzAuMzI2MTcyIDIzLjA5MzYgNi42OTQ2OCAyOS42ODUgMTQuNjY0NSAyOS45ODg0QzguODM4NzMgMjkuNjkxNyA0LjIwMjIyIDI0Ljg0MzUgNC4yMDIyMiAxOC45MDI4WiIgZmlsbD0iIzAwNTNFMCIvPgogIDxwYXRoIGQ9Ik0xNC42NjggMjkuOTg0NEMxNC44NTU2IDI5Ljk5NDUgMTUuMDQ2NSAyOS45OTc5IDE1LjIzNDEgMjkuOTk3OUMxNS4wNDMyIDI5Ljk5NzkgMTQuODU1NiAyOS45OTExIDE0LjY2OCAyOS45ODQ0WiIgZmlsbD0iIzAwNTNFMCIvPgogIDxwYXRoIGQ9Ik0xNS4zNzU2IDAuMDA3ODEyNUMxOC41MTEzIDAuMDg1MzU4IDIxLjAyNzIgMi42NjEyMiAyMS4wMjcyIDUuODMzODRDMjEuMDI3MiA4LjUxMDg0IDE5LjIwMTQgMTAuODM3MiAxNi42MDUxIDExLjUwNDhDMTQuNjI4NSAxMS45NjY3IDEzLjE2NzkgMTMuNTM0NCAxMi44MjYyIDE1LjQwMjNDMTMuODk0OCAxNS4wMDQ0IDE1LjIzMTUgMTUuMDA0NCAxNS4yMzE1IDE1LjAwNDRDMTkuMzQ4OCAxNS4wMDQ0IDIyLjY4MjEgMTEuNjQ2NCAyMi42ODIxIDcuNTA2MTJDMjIuNjgyMSAzLjM2NTg3IDE5LjQyMjUgMC4wODUzNTggMTUuMzc1NiAwLjAwNzgxMjVaIiBmaWxsPSIjRUNFQ0VFIi8+Cjwvc3ZnPgo=",
    "id": "hy3",
    "maxAllowedSize": 192000,
    "maxInputTokens": 192000,
    "maxOutputTokens": 64000,
    "name": "Hy3",
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": false,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "low",
        "high"
      ]
    },
    "relatedModels": {
      "lite": "hy3",
      "reasoning": "hy3"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "tags": [
      "craft"
    ],
    "temperature": 0.9,
    "top_p": 1,
    "vendor": "j"
  },
  {
    "credits": "x0.05",
    "descriptionEn": "Hunyuan's thinking model with enhanced reasoning capabilities",
    "descriptionZh": "混元思考模型，具有增强的推理能力",
    "disabledMultimodal": false,
    "iconUrl": "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxNyIgaGVpZ2h0PSIxNiIgdmlld0JveD0iLTAuNDM3NSAwIDMxLjg3NSAzMCIgZmlsbD0ibm9uZSI+CiAgPHBhdGggZD0iTTE1LjIyODcgMzBDMjMuNDYwMyAzMCAzMC4xMzMzIDIzLjI4NDMgMzAuMTMzMyAxNUMzMC4xMzMzIDYuNzE1NzMgMjMuNDYwMyAwIDE1LjIyODcgMEM2Ljk5NzIgMCAwLjMyNDIxOSA2LjcxNTczIDAuMzI0MjE5IDE1QzAuMzI0MjE5IDIzLjI4NDMgNi45OTcyIDMwIDE1LjIyODcgMzBaIiBmaWxsPSIjQjNEREYyIi8+CiAgPHBhdGggZD0iTTE1LjIzMTggMjkuOTk2NkMxOS4zNDY3IDI5Ljk5NjYgMjIuNjgyNCAyNi42Mzk1IDIyLjY4MjQgMjIuNDk4M0MyMi42ODI0IDE4LjM1NzEgMTkuMzQ2NyAxNSAxNS4yMzE4IDE1QzExLjExNyAxNSA3Ljc4MTI1IDE4LjM1NzEgNy43ODEyNSAyMi40OTgzQzcuNzgxMjUgMjYuNjM5NSAxMS4xMTcgMjkuOTk2NiAxNS4yMzE4IDI5Ljk5NjZaIiBmaWxsPSIjMDA1M0UwIi8+CiAgPG1hc2sgaWQ9Im1hc2swXzYyMV8zMTE1IiBzdHlsZT0ibWFzay10eXBlOmx1bWluYW5jZSIgbWFza1VuaXRzPSJ1c2VyU3BhY2VPblVzZSIgeD0iMCIgeT0iMCIgd2lkdGg9IjMxIiBoZWlnaHQ9IjMwIj4KICAgIDxwYXRoIGQ9Ik0xNS4yMzI2IDMwQzIzLjQ2NDIgMzAgMzAuMTM3MiAyMy4yODQzIDMwLjEzNzIgMTVDMzAuMTM3MiA2LjcxNTczIDIzLjQ2NDIgMCAxNS4yMzI2IDBDNy4wMDExMSAwIDAuMzI4MTI1IDYuNzE1NzMgMC4zMjgxMjUgMTVDMC4zMjgxMjUgMjMuMjg0MyA3LjAwMTExIDMwIDE1LjIzMjYgMzBaIiBmaWxsPSJ3aGl0ZSIvPgogIDwvbWFzaz4KICA8ZyBtYXNrPSJ1cmwoI21hc2swXzYyMV8zMTE1KSI+CiAgICA8cGF0aCBkPSJNMTUuMjMyNCAzMEwzMC4xMzcgMzBMMzAuMTM3IDBMMTUuMjMyNCAwTDE1LjIzMjQgMzBaIiBmaWxsPSIjMDA1M0UwIi8+CiAgPC9nPgogIDxwYXRoIGQ9Ik0xNS4yMzE4IDE0Ljk5NjZDMTkuMzQ2NyAxNC45OTY2IDIyLjY4MjQgMTEuNjM5NSAyMi42ODI0IDcuNDk4MzFDMjIuNjgyNCAzLjM1NzExIDE5LjM0NjcgMCAxNS4yMzE4IDBDMTEuMTE3IDAgNy43ODEyNSAzLjM1NzExIDcuNzgxMjUgNy40OTgzMUM3Ljc4MTI1IDExLjYzOTUgMTEuMTE3IDE0Ljk5NjYgMTUuMjMxOCAxNC45OTY2WiIgZmlsbD0iI0IzRERGMiIvPgogIDxwYXRoIGQ9Ik0xNS4zNzEyIDAuMDA2NzRDMTguNTA2OSAwLjA4NDI5IDIxLjAyMjggMi42NjAxNSAyMS4wMjI4IDUuODMyNzdDMjEuMDIyOCA4LjUwOTc3IDE5LjIyNzEgMTAuODMyOCAxNi42MDA3IDExLjUwMzdDMTQuNDA5NyAxMS45NzU3IDEyLjcyNDYgMTMuOTg1MiAxMi43MjQ2IDE2LjM5MjRDMTIuNzI0NiAxOS4xNjA1IDE0Ljk1MjQgMjEuNDAyNiAxNy43MDI4IDIxLjQwMjZDMTguNDE2NCAyMS40MDI2IDE5LjM4NzkgMjEuMjM0IDIwLjAwMSAyMC45NjA5QzIzLjk2NzUgMTkuMzQ5MyAyNi40NyAxNS40NTUyIDI2LjQ3IDEwLjg4NjdDMjYuNDcgNC44NzE4OCAyMS42MjU4IDAgMTUuNjU1OSAwQzE1LjU1ODggMCAxNS40NjUgMC4wMDMzNyAxNS4zNzEyIDAuMDA2NzRaIiBmaWxsPSIjMkFCOUZGIi8+CiAgPHBhdGggZD0iTTQuMjAyMjIgMTguOTAyOEM0LjIwMjIyIDE1Ljg3MTggNS40MTE2IDEzLjEyNCA3LjM2ODA1IDExLjEyMTNDNy44NzcyNiAxMC41NTQ5IDguMTkyMTcgOS44MTMxMyA4LjE5MjE3IDguOTkzODRDOC4xOTIxNyA3LjI1NzUgNi43OTUxOCA1Ljg1MTU2IDUuMDY5ODkgNS44NTE1NkMzLjg3MDU2IDUuODUxNTYgMi44Mjg2OCA2LjUzNTk5IDIuMzA2MDcgNy41MzczM0MxLjA0OTc5IDkuNzM1NTggMC4zMjYxNzIgMTIuMjgxMSAwLjMyNjE3MiAxNS4wMDE5QzAuMzI2MTcyIDIzLjA5MzYgNi42OTQ2OCAyOS42ODUgMTQuNjY0NSAyOS45ODg0QzguODM4NzMgMjkuNjkxNyA0LjIwMjIyIDI0Ljg0MzUgNC4yMDIyMiAxOC45MDI4WiIgZmlsbD0iIzAwNTNFMCIvPgogIDxwYXRoIGQ9Ik0xNC42NjggMjkuOTg0NEMxNC44NTU2IDI5Ljk5NDUgMTUuMDQ2NSAyOS45OTc5IDE1LjIzNDEgMjkuOTk3OUMxNS4wNDMyIDI5Ljk5NzkgMTQuODU1NiAyOS45OTExIDE0LjY2OCAyOS45ODQ0WiIgZmlsbD0iIzAwNTNFMCIvPgogIDxwYXRoIGQ9Ik0xNS4zNzU2IDAuMDA3ODEyNUMxOC41MTEzIDAuMDg1MzU4IDIxLjAyNzIgMi42NjEyMiAyMS4wMjcyIDUuODMzODRDMjEuMDI3MiA4LjUxMDg0IDE5LjIwMTQgMTAuODM3MiAxNi42MDUxIDExLjUwNDhDMTQuNjI4NSAxMS45NjY3IDEzLjE2NzkgMTMuNTM0NCAxMi44MjYyIDE1LjQwMjNDMTMuODk0OCAxNS4wMDQ0IDE1LjIzMTUgMTUuMDA0NCAxNS4yMzE1IDE1LjAwNDRDMTkuMzQ4OCAxNS4wMDQ0IDIyLjY4MjEgMTEuNjQ2NCAyMi42ODIxIDcuNTA2MTJDMjIuNjgyMSAzLjM2NTg3IDE5LjQyMjUgMC4wODUzNTggMTUuMzc1NiAwLjAwNzgxMjVaIiBmaWxsPSIjRUNFQ0VFIi8+Cjwvc3ZnPgo=",
    "id": "hy3-x",
    "maxAllowedSize": 192000,
    "maxInputTokens": 192000,
    "maxOutputTokens": 64000,
    "name": "Hy3",
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": false,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "low",
        "high"
      ]
    },
    "relatedModels": {
      "lite": "hy3-x",
      "reasoning": "hy3-x"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "tags": [
      "craft"
    ],
    "temperature": 0.9,
    "top_p": 1,
    "vendor": "j"
  },
  {
    "contextWindow": {
      "defaultLength": 300000,
      "supportedLengths": [
        300000,
        1000000
      ]
    },
    "credits": "x0.29",
    "descriptionEn": "Hunyuan's thinking model with enhanced reasoning capabilities",
    "descriptionZh": "混元思考模型，具有增强的推理能力",
    "disabledMultimodal": false,
    "id": "hy4-preview",
    "maxAllowedSize": 1000000,
    "maxInputTokens": 1000000,
    "maxOutputTokens": 64000,
    "name": "Hy4 preview",
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": false,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "high"
      ]
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "temperature": 0.9,
    "top_p": 1,
    "vendor": "j"
  },
  {
    "contextWindow": {
      "defaultLength": 300000,
      "supportedLengths": [
        300000,
        1000000
      ]
    },
    "credits": "x0.29",
    "descriptionEn": "Hunyuan's thinking model with enhanced reasoning capabilities",
    "descriptionZh": "混元思考模型，具有增强的推理能力",
    "disabledMultimodal": false,
    "id": "hy4-preview-dev",
    "maxAllowedSize": 1000000,
    "maxInputTokens": 1000000,
    "maxOutputTokens": 64000,
    "name": "Hy4 preview",
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": false,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "high"
      ]
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "temperature": 0.9,
    "top_p": 1,
    "vendor": "j"
  },
  {
    "contextWindow": {
      "defaultLength": 300000,
      "supportedLengths": [
        300000,
        1000000
      ]
    },
    "credits": "x0.00",
    "descriptionEn": "Hunyuan's thinking model with enhanced reasoning capabilities",
    "descriptionZh": "混元思考模型，具有增强的推理能力",
    "disabledMultimodal": false,
    "id": "hy4-preview-f",
    "maxAllowedSize": 1000000,
    "maxInputTokens": 1000000,
    "maxOutputTokens": 64000,
    "name": "Hy4 preview",
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": false,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "high"
      ]
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "temperature": 0.9,
    "top_p": 1,
    "vendor": "j"
  },
  {
    "contextWindow": {
      "defaultLength": 300000,
      "supportedLengths": [
        300000,
        1000000
      ]
    },
    "credits": "x0.29",
    "descriptionEn": "Hunyuan's thinking model with enhanced reasoning capabilities",
    "descriptionZh": "混元思考模型，具有增强的推理能力",
    "disabledMultimodal": false,
    "id": "hy4-preview-x",
    "maxAllowedSize": 1000000,
    "maxInputTokens": 1000000,
    "maxOutputTokens": 64000,
    "name": "Hy4 preview",
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": false,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "high"
      ]
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "temperature": 0.9,
    "top_p": 1,
    "vendor": "j"
  },
  {
    "credits": "x0.18",
    "descriptionEn": "Well-rounded model for everyday use",
    "descriptionZh": "能力均衡，适合日常使用",
    "disabledMultimodal": false,
    "id": "minimax-m2.5",
    "maxAllowedSize": 200000,
    "maxInputTokens": 200000,
    "maxOutputTokens": 48000,
    "name": "MiniMax-M2.5",
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "temperature": 1,
    "vendor": "f"
  },
  {
    "credits": "x0.71",
    "descriptionEn": "Native Multimodal Model",
    "descriptionZh": "原生多模态模型",
    "id": "glm-5v-turbo",
    "maxAllowedSize": 200000,
    "maxInputTokens": 200000,
    "maxOutputTokens": 64000,
    "name": "GLM-5v-Turbo",
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "relatedModels": {
      "lite": "glm-5v-turbo",
      "reasoning": "glm-5v-turbo"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "tags": [
      "craft"
    ],
    "temperature": 1,
    "vendor": "e"
  },
  {
    "contextWindow": {
      "defaultLength": 300000,
      "supportedLengths": [
        300000,
        1000000
      ]
    },
    "credits": "x0.79",
    "descriptionEn": "Great for daily use",
    "descriptionZh": "能力均衡，适合日常使用",
    "id": "glm-5.3",
    "maxAllowedSize": 1000000,
    "maxInputTokens": 1000000,
    "maxOutputTokens": 48000,
    "name": "GLM-5.3",
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": true,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "low",
        "high",
        "max"
      ]
    },
    "relatedModels": {
      "lite": "glm-5.3",
      "reasoning": "glm-5.3"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "tags": [
      "craft"
    ],
    "temperature": 1,
    "vendor": "e"
  },
  {
    "contextWindow": {
      "defaultLength": 300000,
      "supportedLengths": [
        300000,
        1000000
      ]
    },
    "credits": "x0.06",
    "descriptionEn": "Native multimodal, excelling at complex, long-horizon autonomous tasks.",
    "descriptionZh": "原生多模态，擅长处理复杂的长程自主任务。",
    "id": "glm-5.3-flash",
    "maxAllowedSize": 1000000,
    "maxInputTokens": 1000000,
    "maxOutputTokens": 32000,
    "name": "GLM-5.3-Flash",
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": true,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "low",
        "high",
        "max"
      ]
    },
    "relatedModels": {
      "lite": "glm-5.3-flash",
      "reasoning": "glm-5.3-flash"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "temperature": 1,
    "vendor": "f"
  },
  {
    "contextWindow": {
      "defaultLength": 300000,
      "supportedLengths": [
        300000,
        1000000
      ]
    },
    "credits": "x0.79",
    "descriptionEn": "1M context, built for long-horizon tasks.",
    "descriptionZh": "1M 上下文，擅长长程任务",
    "id": "glm-5.2",
    "maxAllowedSize": 1000000,
    "maxInputTokens": 1000000,
    "maxOutputTokens": 48000,
    "name": "GLM-5.2",
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": true,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "high",
        "xhigh"
      ]
    },
    "relatedModels": {
      "lite": "glm-5.2",
      "reasoning": "glm-5.2"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "tags": [
      "craft"
    ],
    "temperature": 1,
    "vendor": "e"
  },
  {
    "credits": "x0.79",
    "descriptionEn": "Great for daily use",
    "descriptionZh": "能力均衡，适合日常使用",
    "id": "glm-5.1",
    "maxAllowedSize": 200000,
    "maxInputTokens": 200000,
    "maxOutputTokens": 48000,
    "name": "GLM-5.1",
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "relatedModels": {
      "lite": "glm-5.1",
      "reasoning": "glm-5.1"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "tags": [
      "craft"
    ],
    "temperature": 1,
    "vendor": "e"
  },
  {
    "credits": "x0.95",
    "descriptionEn": "Deeply optimized for agent scenario",
    "descriptionZh": "面向 Agent 场景进行了深度优化",
    "id": "glm-5.0-turbo",
    "maxAllowedSize": 200000,
    "maxInputTokens": 200000,
    "maxOutputTokens": 48000,
    "name": "GLM-5.0-Turbo",
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "relatedModels": {
      "lite": "glm-5.0-turbo",
      "reasoning": "glm-5.0-turbo"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "tags": [
      "craft"
    ],
    "temperature": 1,
    "vendor": "e"
  },
  {
    "credits": "x0.11",
    "descriptionEn": "GLM-4.6V multimodal model",
    "descriptionZh": "GLM-4.6V 多模态模型，支持图片输入",
    "id": "glm-4.6v",
    "maxAllowedSize": 128000,
    "maxInputTokens": 128000,
    "maxOutputTokens": 32000,
    "name": "GLM-4.6V",
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "temperature": 1,
    "vendor": "f"
  },
  {
    "contextWindow": {
      "defaultLength": 300000,
      "supportedLengths": [
        300000,
        1000000
      ]
    },
    "credits": "x1.62",
    "descriptionEn": "Excels at complex, long-horizon autonomous tasks, with standout front-end skills and strong knowledge work and scientific reasoning",
    "descriptionZh": "擅长处理复杂的长程自主任务，前端开发能力突出，同时在知识工作与科研推理上表现出色。",
    "id": "kimi-k3-1",
    "maxAllowedSize": 1000000,
    "maxInputTokens": 1000000,
    "maxOutputTokens": 32000,
    "name": "Kimi-K3",
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": true,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "low",
        "high",
        "xhigh"
      ]
    },
    "relatedModels": {
      "lite": "kimi-k3-1",
      "reasoning": "kimi-k3-1"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "temperature": 1,
    "vendor": "f"
  },
  {
    "canDisableThinking": true,
    "contextWindow": {
      "defaultLength": 300000,
      "supportedLengths": [
        300000,
        1000000
      ]
    },
    "credits": "x0.77",
    "descriptionEn": "Excels at complex, long-horizon autonomous tasks, with standout front-end skills and strong knowledge work and scientific reasoning",
    "descriptionZh": "擅长处理复杂的长程自主任务，前端开发能力突出，同时在知识工作与科研推理上表现出色。",
    "id": "kimi-k2.8-preview",
    "maxAllowedSize": 1000000,
    "maxInputTokens": 1000000,
    "maxOutputTokens": 32000,
    "name": "Kimi-K2.8-Preview",
    "onlyReasoning": true,
    "reasoning": {
      "canDisableThinking": true,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "low",
        "high",
        "max"
      ]
    },
    "relatedModels": {
      "lite": "kimi-k2.8-preview",
      "reasoning": "kimi-k2.8-preview"
    },
    "summary": "auto",
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "temperature": 1,
    "vendor": "f"
  },
  {
    "credits": "x0.57",
    "descriptionEn": "A multimodal model, good for daily use.",
    "descriptionZh": "多模态模型，适合日常任务",
    "id": "kimi-k2.7",
    "maxAllowedSize": 256000,
    "maxInputTokens": 256000,
    "maxOutputTokens": 32000,
    "name": "Kimi-K2.7-Code",
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "relatedModels": {
      "lite": "kimi-k2.7",
      "reasoning": "kimi-k2.7"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "temperature": 1,
    "vendor": "f"
  },
  {
    "credits": "x0.52",
    "descriptionEn": "A multimodal model, good for daily use.",
    "descriptionZh": "多模态模型，适合日常任务",
    "id": "kimi-k2.6",
    "maxAllowedSize": 256000,
    "maxInputTokens": 256000,
    "maxOutputTokens": 32000,
    "name": "Kimi-K2.6",
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "relatedModels": {
      "lite": "kimi-k2.6",
      "reasoning": "kimi-k2.6"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "tags": [
      "craft"
    ],
    "temperature": 1,
    "vendor": "f"
  },
  {
    "credits": "x0.45",
    "descriptionEn": "A multimodal model, good for daily use.",
    "descriptionZh": "多模态模型，适合日常任务",
    "id": "kimi-k2.5",
    "maxAllowedSize": 256000,
    "maxInputTokens": 256000,
    "maxOutputTokens": 32000,
    "name": "Kimi-K2.5",
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "tags": [
      "craft"
    ],
    "temperature": 1,
    "vendor": "f"
  },
  {
    "credits": "x0.54",
    "descriptionEn": "Good for complex coding tasks",
    "descriptionZh": "适合复杂编码任务",
    "disabledMultimodal": false,
    "id": "kimi-k2-thinking",
    "maxAllowedSize": 256000,
    "maxInputTokens": 256000,
    "maxOutputTokens": 32000,
    "name": "Kimi-K2-Thinking",
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "temperature": 1,
    "vendor": "f"
  },
  {
    "contextWindow": {
      "defaultLength": 300000,
      "supportedLengths": [
        300000,
        512000
      ]
    },
    "credits": "x0.25",
    "descriptionEn": "Native multimodal model for coding and agents tasks",
    "descriptionZh": "原生多模态，擅长代码、智能体任务",
    "disabledMultimodal": false,
    "id": "minimax-m3",
    "maxAllowedSize": 512000,
    "maxInputTokens": 512000,
    "maxOutputTokens": 128000,
    "name": "MiniMax-M3",
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "relatedModels": {
      "lite": "minimax-m3",
      "reasoning": "minimax-m3"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "temperature": 1,
    "vendor": "f"
  },
  {
    "credits": "x0.26",
    "descriptionEn": "Well-rounded model for everyday use",
    "descriptionZh": "能力均衡，适合日常使用",
    "disabledMultimodal": false,
    "id": "minimax-m2.7",
    "maxAllowedSize": 200000,
    "maxInputTokens": 200000,
    "maxOutputTokens": 48000,
    "name": "MiniMax-M2.7",
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "relatedModels": {
      "lite": "minimax-m2.7",
      "reasoning": "minimax-m2.7"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "tags": [
      "craft"
    ],
    "temperature": 1,
    "vendor": "f"
  },
  {
    "credits": "x0.23",
    "descriptionEn": "Advanced language model with strong reasoning capabilities",
    "descriptionZh": "具有强大推理能力的先进语言模型",
    "disabledMultimodal": false,
    "id": "glm-4.6",
    "maxAllowedSize": 168000,
    "maxInputTokens": 168000,
    "maxOutputTokens": 32000,
    "name": "GLM-4.6",
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "temperature": 1,
    "vendor": "f"
  },
  {
    "contextWindow": {
      "defaultLength": 300000,
      "supportedLengths": [
        300000,
        1000000
      ]
    },
    "credits": "x0.17",
    "descriptionEn": "DeepSeek flagship model, supporting 1M context window",
    "descriptionZh": "DeepSeek 旗舰模型，支持 1M 上下文窗口",
    "id": "deepseek-v4-flash",
    "maxAllowedSize": 1000000,
    "maxInputTokens": 1000000,
    "maxOutputTokens": 50000,
    "name": "Deepseek-V4-Flash",
    "onlyReasoning": false,
    "reasoning": {
      "canDisableThinking": true,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "high",
        "xhigh"
      ]
    },
    "relatedModels": {
      "lite": "deepseek-v4-flash",
      "reasoning": "deepseek-v4-flash"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "temperature": 1,
    "vendor": "f"
  },
  {
    "contextWindow": {
      "defaultLength": 300000,
      "supportedLengths": [
        300000,
        1000000
      ]
    },
    "credits": "x0.11",
    "descriptionEn": "DeepSeek flagship model, supporting 1M context window, native multimodal model",
    "descriptionZh": "DeepSeek 旗舰模型，支持 1M 上下文窗口，原生多模态",
    "id": "deepseek-v4.1-flash",
    "maxAllowedSize": 1000000,
    "maxInputTokens": 1000000,
    "maxOutputTokens": 128000,
    "name": "Deepseek-V4.1-Flash",
    "onlyReasoning": true,
    "reasoning": {
      "defaultEffort": "high",
      "supportedEfforts": [
        "low",
        "high",
        "max"
      ],
      "summary": "auto"
    },
    "relatedModels": {
      "lite": "deepseek-v4.1-flash",
      "reasoning": "deepseek-v4.1-flash"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "temperature": 1,
    "vendor": "f"
  },
  {
    "contextWindow": {
      "defaultLength": 300000,
      "supportedLengths": [
        300000,
        1000000
      ]
    },
    "credits": "x0.51",
    "descriptionEn": "DeepSeek flagship model, supporting 1M context window",
    "descriptionZh": "DeepSeek 旗舰模型，支持 1M 上下文窗口",
    "id": "deepseek-v4-pro",
    "maxAllowedSize": 1000000,
    "maxInputTokens": 1000000,
    "maxOutputTokens": 50000,
    "name": "Deepseek-V4-Pro",
    "onlyReasoning": false,
    "reasoning": {
      "canDisableThinking": true,
      "defaultEffort": "high",
      "summary": "auto",
      "supportedEfforts": [
        "high",
        "xhigh"
      ]
    },
    "relatedModels": {
      "lite": "deepseek-v4-pro",
      "reasoning": "deepseek-v4-pro"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "temperature": 1,
    "vendor": "f"
  },
  {
    "credits": "x0.29",
    "descriptionEn": "DeepSeek-V3.2, good for daily use",
    "descriptionZh": "DeepSeek-V3.2 模型，适合日常使用",
    "disabledMultimodal": false,
    "id": "deepseek-v3-2-volc",
    "maxAllowedSize": 96000,
    "maxInputTokens": 96000,
    "maxOutputTokens": 32000,
    "name": "DeepSeek-V3.2",
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "relatedModels": {
      "lite": "deepseek-v3-2-volc",
      "reasoning": "deepseek-v3-2-volc"
    },
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "tags": [
      "craft"
    ],
    "temperature": 1,
    "vendor": "f"
  },
  {
    "credits": "x0.52",
    "descriptionEn": "DeepSeek's flagship model, good for planning, debugging, coding, and more",
    "descriptionZh": "DeepSeek 的旗舰模型，适合规划、调试、编码等任务",
    "disabledMultimodal": false,
    "id": "deepseek-v3-1-volc",
    "maxAllowedSize": 128000,
    "maxInputTokens": 96000,
    "maxOutputTokens": 32000,
    "name": "DeepSeek-V3-1-Terminus",
    "relatedModels": {
      "lite": "deepseek-v3-1-volc",
      "reasoning": "deepseek-v3-1-volc"
    },
    "supportsImages": true,
    "supportsToolCall": true,
    "temperature": 0.8,
    "vendor": "f"
  },
  {
    "credits": "x0.52",
    "descriptionEn": "DeepSeek's flagship model, good for planning, debugging, coding, and more",
    "descriptionZh": "DeepSeek 的旗舰模型，适合规划、调试、编码等任务",
    "disabledMultimodal": false,
    "id": "deepseek-v3-1-lkeap",
    "maxAllowedSize": 128000,
    "maxInputTokens": 96000,
    "maxOutputTokens": 32000,
    "name": "DeepSeek-V3-1",
    "supportsImages": true,
    "supportsToolCall": true,
    "temperature": 0.8,
    "vendor": "f"
  },
  {
    "credits": "x0.52",
    "descriptionEn": "DeepSeek's flagship model, good for planning, debugging, coding, and more",
    "descriptionZh": "DeepSeek 的旗舰模型，适合规划、调试、编码等任务",
    "disabledMultimodal": false,
    "id": "deepseek-v3-1",
    "maxAllowedSize": 128000,
    "maxInputTokens": 96000,
    "maxOutputTokens": 32000,
    "name": "DeepSeek-V3.1",
    "supportsImages": true,
    "supportsToolCall": true,
    "temperature": 0.8,
    "vendor": "f"
  },
  {
    "credits": "x0.52",
    "descriptionEn": "DeepSeek's flagship model, good for planning, debugging, coding, and more",
    "descriptionZh": "DeepSeek 的旗舰模型，适合规划、调试、编码等任务",
    "disabledMultimodal": false,
    "id": "deepseek-v3-0324-lkeap",
    "maxAllowedSize": 128000,
    "maxInputTokens": 112000,
    "maxOutputTokens": 16000,
    "name": "DeepSeek-V3-0324",
    "supportsImages": true,
    "supportsToolCall": true,
    "temperature": 0.8,
    "vendor": "f"
  },
  {
    "credits": "",
    "descriptionEn": "Open-source reasoning model from DeepSeek, optimised for logic & math",
    "descriptionZh": "DeepSeek 的开源推理模型，专为逻辑与数学优化",
    "disabledMultimodal": false,
    "id": "deepseek-r1-0528-lkeap",
    "maxAllowedSize": 112000,
    "maxInputTokens": 96000,
    "maxOutputTokens": 16000,
    "name": "DeepSeek-R1-0528",
    "supportsImages": true,
    "supportsToolCall": true,
    "temperature": 0.8,
    "vendor": "f"
  },
  {
    "credits": "",
    "descriptionEn": "Moonshot AI's conversational model with strong Chinese language capabilities",
    "descriptionZh": "月之暗面的对话模型，具有强大的中文语言能力",
    "disabledMultimodal": false,
    "id": "kimi-k2-instruct-taiji",
    "maxAllowedSize": 31000,
    "maxInputTokens": 31000,
    "maxOutputTokens": 8192,
    "name": "Kimi-K2",
    "supportsImages": true,
    "supportsToolCall": true,
    "vendor": "f"
  },
  {
    "credits": "",
    "descriptionEn": "Great for daily use, good at most things",
    "descriptionZh": "适合日常使用，在大多数任务上表现良好",
    "id": "default-1.1",
    "maxAllowedSize": 200000,
    "maxInputTokens": 200000,
    "maxOutputTokens": 8192,
    "name": "Claude-3.7-Sonnet",
    "onlyReasoning": true,
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "vendor": "e"
  },
  {
    "credits": "",
    "descriptionEn": "Open-source reasoning model from DeepSeek, optimised for logic & math",
    "descriptionZh": "DeepSeek 的开源推理模型，专为逻辑与数学优化",
    "id": "deepseek-r1-0528",
    "maxAllowedSize": 96000,
    "maxInputTokens": 96000,
    "maxOutputTokens": 8192,
    "name": "deepseek-r1",
    "supportsExtra": true,
    "vendor": "tencent"
  },
  {
    "credits": "",
    "descriptionEn": "DeepSeek's flagship model, good for planning, debugging, coding, and more",
    "descriptionZh": "DeepSeek 的旗舰模型，适合规划、调试、编码等任务",
    "id": "deepseek-v3-0324",
    "maxAllowedSize": 96000,
    "maxInputTokens": 96000,
    "maxOutputTokens": 8192,
    "name": "deepseek-v3",
    "vendor": "tencent"
  },
  {
    "credits": "",
    "descriptionEn": "Great for daily use, good at most things",
    "descriptionZh": "适合日常使用，在大多数任务上表现良好",
    "id": "default-1.2",
    "maxAllowedSize": 200000,
    "maxInputTokens": 200000,
    "maxOutputTokens": 24000,
    "name": "Claude-4.0-Sonnet",
    "onlyReasoning": true,
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "vendor": "e"
  },
  {
    "credits": "",
    "disabledMultimodal": false,
    "iconUrl": "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxNyIgaGVpZ2h0PSIxNiIgdmlld0JveD0iLTAuNDM3NSAwIDMxLjg3NSAzMCIgZmlsbD0ibm9uZSI+CiAgPHBhdGggZD0iTTE1LjIyODcgMzBDMjMuNDYwMyAzMCAzMC4xMzMzIDIzLjI4NDMgMzAuMTMzMyAxNUMzMC4xMzMzIDYuNzE1NzMgMjMuNDYwMyAwIDE1LjIyODcgMEM2Ljk5NzIgMCAwLjMyNDIxOSA2LjcxNTczIDAuMzI0MjE5IDE1QzAuMzI0MjE5IDIzLjI4NDMgNi45OTcyIDMwIDE1LjIyODcgMzBaIiBmaWxsPSIjQjNEREYyIi8+CiAgPHBhdGggZD0iTTE1LjIzMTggMjkuOTk2NkMxOS4zNDY3IDI5Ljk5NjYgMjIuNjgyNCAyNi42Mzk1IDIyLjY4MjQgMjIuNDk4M0MyMi42ODI0IDE4LjM1NzEgMTkuMzQ2NyAxNSAxNS4yMzE4IDE1QzExLjExNyAxNSA3Ljc4MTI1IDE4LjM1NzEgNy43ODEyNSAyMi40OTgzQzcuNzgxMjUgMjYuNjM5NSAxMS4xMTcgMjkuOTk2NiAxNS4yMzE4IDI5Ljk5NjZaIiBmaWxsPSIjMDA1M0UwIi8+CiAgPG1hc2sgaWQ9Im1hc2swXzYyMV8zMTE1IiBzdHlsZT0ibWFzay10eXBlOmx1bWluYW5jZSIgbWFza1VuaXRzPSJ1c2VyU3BhY2VPblVzZSIgeD0iMCIgeT0iMCIgd2lkdGg9IjMxIiBoZWlnaHQ9IjMwIj4KICAgIDxwYXRoIGQ9Ik0xNS4yMzI2IDMwQzIzLjQ2NDIgMzAgMzAuMTM3MiAyMy4yODQzIDMwLjEzNzIgMTVDMzAuMTM3MiA2LjcxNTczIDIzLjQ2NDIgMCAxNS4yMzI2IDBDNy4wMDExMSAwIDAuMzI4MTI1IDYuNzE1NzMgMC4zMjgxMjUgMTVDMC4zMjgxMjUgMjMuMjg0MyA3LjAwMTExIDMwIDE1LjIzMjYgMzBaIiBmaWxsPSJ3aGl0ZSIvPgogIDwvbWFzaz4KICA8ZyBtYXNrPSJ1cmwoI21hc2swXzYyMV8zMTE1KSI+CiAgICA8cGF0aCBkPSJNMTUuMjMyNCAzMEwzMC4xMzcgMzBMMzAuMTM3IDBMMTUuMjMyNCAwTDE1LjIzMjQgMzBaIiBmaWxsPSIjMDA1M0UwIi8+CiAgPC9nPgogIDxwYXRoIGQ9Ik0xNS4yMzE4IDE0Ljk5NjZDMTkuMzQ2NyAxNC45OTY2IDIyLjY4MjQgMTEuNjM5NSAyMi42ODI0IDcuNDk4MzFDMjIuNjgyNCAzLjM1NzExIDE5LjM0NjcgMCAxNS4yMzE4IDBDMTEuMTE3IDAgNy43ODEyNSAzLjM1NzExIDcuNzgxMjUgNy40OTgzMUM3Ljc4MTI1IDExLjYzOTUgMTEuMTE3IDE0Ljk5NjYgMTUuMjMxOCAxNC45OTY2WiIgZmlsbD0iI0IzRERGMiIvPgogIDxwYXRoIGQ9Ik0xNS4zNzEyIDAuMDA2NzRDMTguNTA2OSAwLjA4NDI5IDIxLjAyMjggMi42NjAxNSAyMS4wMjI4IDUuODMyNzdDMjEuMDIyOCA4LjUwOTc3IDE5LjIyNzEgMTAuODMyOCAxNi42MDA3IDExLjUwMzdDMTQuNDA5NyAxMS45NzU3IDEyLjcyNDYgMTMuOTg1MiAxMi43MjQ2IDE2LjM5MjRDMTIuNzI0NiAxOS4xNjA1IDE0Ljk1MjQgMjEuNDAyNiAxNy43MDI4IDIxLjQwMjZDMTguNDE2NCAyMS40MDI2IDE5LjM4NzkgMjEuMjM0IDIwLjAwMSAyMC45NjA5QzIzLjk2NzUgMTkuMzQ5MyAyNi40NyAxNS40NTUyIDI2LjQ3IDEwLjg4NjdDMjYuNDcgNC44NzE4OCAyMS42MjU4IDAgMTUuNjU1OSAwQzE1LjU1ODggMCAxNS40NjUgMC4wMDMzNyAxNS4zNzEyIDAuMDA2NzRaIiBmaWxsPSIjMkFCOUZGIi8+CiAgPHBhdGggZD0iTTQuMjAyMjIgMTguOTAyOEM0LjIwMjIyIDE1Ljg3MTggNS40MTE2IDEzLjEyNCA3LjM2ODA1IDExLjEyMTNDNy44NzcyNiAxMC41NTQ5IDguMTkyMTcgOS44MTMxMyA4LjE5MjE3IDguOTkzODRDOC4xOTIxNyA3LjI1NzUgNi43OTUxOCA1Ljg1MTU2IDUuMDY5ODkgNS44NTE1NkMzLjg3MDU2IDUuODUxNTYgMi44Mjg2OCA2LjUzNTk5IDIuMzA2MDcgNy41MzczM0MxLjA0OTc5IDkuNzM1NTggMC4zMjYxNzIgMTIuMjgxMSAwLjMyNjE3MiAxNS4wMDE5QzAuMzI2MTcyIDIzLjA5MzYgNi42OTQ2OCAyOS42ODUgMTQuNjY0NSAyOS45ODg0QzguODM4NzMgMjkuNjkxNyA0LjIwMjIyIDI0Ljg0MzUgNC4yMDIyMiAxOC45MDI4WiIgZmlsbD0iIzAwNTNFMCIvPgogIDxwYXRoIGQ9Ik0xNC42NjggMjkuOTg0NEMxNC44NTU2IDI5Ljk5NDUgMTUuMDQ2NSAyOS45OTc5IDE1LjIzNDEgMjkuOTk3OUMxNS4wNDMyIDI5Ljk5NzkgMTQuODU1NiAyOS45OTExIDE0LjY2OCAyOS45ODQ0WiIgZmlsbD0iIzAwNTNFMCIvPgogIDxwYXRoIGQ9Ik0xNS4zNzU2IDAuMDA3ODEyNUMxOC41MTEzIDAuMDg1MzU4IDIxLjAyNzIgMi42NjEyMiAyMS4wMjcyIDUuODMzODRDMjEuMDI3MiA4LjUxMDg0IDE5LjIwMTQgMTAuODM3MiAxNi42MDUxIDExLjUwNDhDMTQuNjI4NSAxMS45NjY3IDEzLjE2NzkgMTMuNTM0NCAxMi44MjYyIDE1LjQwMjNDMTMuODk0OCAxNS4wMDQ0IDE1LjIzMTUgMTUuMDA0NCAxNS4yMzE1IDE1LjAwNDRDMTkuMzQ4OCAxNS4wMDQ0IDIyLjY4MjEgMTEuNjQ2NCAyMi42ODIxIDcuNTA2MTJDMjIuNjgyMSAzLjM2NTg3IDE5LjQyMjUgMC4wODUzNTggMTUuMzc1NiAwLjAwNzgxMjVaIiBmaWxsPSIjRUNFQ0VFIi8+Cjwvc3ZnPgo=",
    "id": "hunyuan-2.0-instruct",
    "maxAllowedSize": 128000,
    "maxInputTokens": 128000,
    "maxOutputTokens": 16000,
    "name": "Hunyuan-2.0-Instruct",
    "onlyReasoning": true,
    "reasoning": {
      "effort": "medium",
      "summary": "auto"
    },
    "repetition_penalty": 1.05,
    "supportsImages": true,
    "supportsReasoning": true,
    "supportsToolCall": true,
    "temperature": 0.7,
    "top_k": 20,
    "top_p": 0.8,
    "vendor": "j"
  },
  {
    "credits": "",
    "descriptionEn": "Tencent's lightweight, fast general-purpose model",
    "descriptionZh": "腾讯自研的轻量、快速的通用模型",
    "disabledMultimodal": false,
    "id": "hunyuan-chat",
    "maxAllowedSize": 128000,
    "maxInputTokens": 128000,
    "maxOutputTokens": 8192,
    "name": "Hunyuan-Turbos",
    "supportsImages": true,
    "supportsToolCall": true,
    "vendor": "j"
  }
]
'''

STATIC_CN_MODELS = json.loads(_CN_JSON)
