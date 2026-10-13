# VSCode Copilot 一次性配置助手

## 用户操作

1. 使用 VS Code 1.141 或更新版本，在**默认配置文件**中启用内置 GitHub Copilot。扩展页面搜索 `@builtin copilot` 可以找到它。保存正在编辑的模型配置。
2. 在 ezcpa 的 **VSCode Copilot** 页面选择模型，点击启用或更新配置。
3. 首次配置或更换 CPA API Key 时，会出现一个 **Extension Development Host / 扩展开发宿主** 临时窗口。它是 ezcpa 的一次性助手，不是要求安装的插件。保持 ezcpa 打开，等待最多两分钟，不要在临时窗口里编辑项目。
4. 助手完成后会自动关闭自己的窗口。回到平常使用的 VS Code，在 Copilot 模型列表中选择 **CPA (EasyCLIProxyAPI)** 下的模型。

日常聊天直接使用内置 Copilot 的 **Custom Endpoint**。助手不处理聊天请求，不常驻、不监听文件、不加入扩展安装列表。修改模型或端口会复用已保存的密钥引用；更换 API Key 时再次运行助手。

## 排查指令

以下命令只读取状态，不修改配置：

```shell
code --version
code --list-extensions
```

一次性助手不会出现在第二条命令的输出中，内置 Copilot 也可能不显示。如果 `code` 不在 PATH 中，可以直接运行 VS Code 的启动程序，或在 ezcpa“配置管理”中指定它。

Windows PowerShell 示例（将路径替换为自己的安装路径）：

```powershell
& 'C:\Path\To\Microsoft VS Code\bin\code.cmd' --version
```

- **Copilot 未启用**：在默认配置文件的扩展页面搜索 `@builtin copilot`，启用 GitHub Copilot；如果关闭了 AI 功能，也需先开启，再回 ezcpa 重试。
- **版本不支持**：更新 VS Code，不要修改 VS Code 程序文件或密钥数据库。
- **模型配置未保存**：保存 `chatLanguageModels.json` 后重试。
- **配置文件不匹配**：在 VS Code 的“配置文件 / Profiles”中切回默认配置文件后重试。当前不支持命名配置文件、远程配置文件和自定义用户数据目录。
- **超时或窗口被提前关闭**：关闭遗留的“扩展开发宿主”窗口，确认以上条件后重试。不要关闭正常工作的编辑器窗口；无需安装或卸载额外插件。

## 实现与限制

ezcpa 把内嵌的助手源码写入用户配置目录下的独立临时运行目录，然后调用：

```shell
code --new-window --extensionDevelopmentPath=<本次助手目录>
```

这使用 VS Code 的临时扩展开发宿主机制，不调用 `--install-extension`，不产生 VSIX，也不将助手写入用户的扩展目录。助手仅允许在 `ExtensionMode.Development` 中运行；即使有人手动安装它，也不会执行配置或关闭正常编辑器。

助手读取本次短期有效的本地请求后立即删除其中的明文密钥文件。它调用 VS Code 原生 `lm.addLanguageModelsProviderGroup` 保存密钥，返回 `${input:chat.lm.secret.…}` 引用，再移除自己的临时模型分组。真正的 CPA 模型配置由 ezcpa 原有的配置事务和备份流程写入，其他供应商分组予以保留。

成功、失败或超时后，ezcpa 清理本次已知临时文件；助手关闭自己的开发宿主窗口，不结束用户的正常 VS Code 进程。若 ezcpa 和助手同时崩溃，可能留下临时文件或窗口；过期请求不会再执行。密钥不会放进启动参数、响应或日志，不直接读写 VS Code 的加密数据库。持久化缓存只包含密钥摘要及引用。

助手退出后，VS Code 加密保存的密钥和 CPA 模型配置继续有效。关闭 CPA 配置时，ezcpa 移除或恢复其模型分组；原生密钥存储中的引用对应项可能保留。显示已配置表示配置和引用匹配，不表示已验证上游模型可用性。
