# Privacy policy: rex: Page to JSON

Effective 2026-11-07. Applies to the Chrome extension "rex: Page to JSON" (version 0.3.0 and later).
[中文版本在下方](#隐私政策rex网页转-json)

## Summary

rex turns the page you are viewing into a JSON record on your own computer. It never connects to the
internet, has no account, no analytics and no advertising, and sends nothing to its developer or anyone
else.

## What rex reads

- **The page in the current tab, only when you click the rex icon** (or press its keyboard shortcut). Chrome's
  `activeTab` permission grants access to that one tab at that moment. rex asks for no access to any website.
- It reads the page's HTML, extracts the record with recipes built into the extension, and shows it in the
  popup. The page and the record stay in your browser's memory and are discarded when the popup closes.

## What rex stores

In Chrome's extension storage, on your computer only:

- your settings: what Copy JSON copies, developer mode, and a recipe you paste in developer mode;
- which popup view you used last (Reader or Table).

rex keeps no history of the pages you open or the records it extracts. Removing the extension deletes this
storage.

## What leaves your computer

Nothing, unless you do it yourself:

- **Copy JSON and Download** put the record on your clipboard or in a file you choose.
- **Report broken recipe** opens a GitHub "new issue" form in a new tab, filled with the recipe's name and
  version, the extraction status and the extension's version. The page address is included only if you tick
  "include page URL in the report". Nothing is sent until you submit the form on GitHub, whose own privacy policy
  then applies.
- **Save as fixture** (developer mode) writes a copy of the page, with its text replaced, to a file on your
  computer.

The extension's content security policy lets it connect only to its own files, and every automated test run
checks that it makes no network request.

## What rex does not do

It does not sell, share or transfer data; does not use data for advertising, credit decisions or any purpose
other than showing you the record of the page you asked for; and does not run remote code (the recipes and
the WebAssembly engine are inside the extension package).

## Children

rex is a general-purpose tool and does not knowingly handle data of children.

## Changes and contact

Changes to this policy are published in this file, with a new effective date:
<https://github.com/Shuozeli/rex/blob/main/PRIVACY.md>. Questions: open an issue at
<https://github.com/Shuozeli/rex/issues>.

---

# 隐私政策：rex：网页转 JSON

生效日期 2026-11-07。适用于 Chrome 扩展「rex：网页转 JSON」0.3.0 及以后的版本。

## 概要

rex 在你自己的电脑上把正在浏览的网页变成一条 JSON 记录。它从不联网，没有账号、没有统计分析、没有广告，
不向开发者或任何人发送任何数据。

## rex 读取什么

- **只在你点击 rex 图标（或按快捷键）时，读取当前标签页的网页。** Chrome 的 `activeTab` 权限只在那一刻
  授予这一个标签页的访问权。rex 不申请任何网站的访问权限。
- 它读取网页的 HTML，用扩展内置的配方提取记录，在弹窗中显示。网页和记录只在浏览器内存中，弹窗关闭即丢弃。

## rex 保存什么

只保存在你电脑上的 Chrome 扩展存储中：

- 你的设置：「复制 JSON」复制的内容、开发者模式、开发者模式下粘贴的配方；
- 上次使用的弹窗视图（阅读或数据表）。

rex 不记录你打开过的网页，也不保存提取过的记录。卸载扩展即删除这些数据。

## 什么会离开你的电脑

除非你自己操作，否则什么都不会：

- **复制 JSON 和下载**：把记录放到剪贴板或你选择的文件中。
- **报告配方失效**：在新标签页打开 GitHub 的新建 issue 表单，预填配方名称和版本、提取状态和扩展版本。
  只有勾选「在报告中附上页面网址」时才包含网页地址。在 GitHub 上提交表单之前不会发送任何内容，提交后适用
  GitHub 自己的隐私政策。
- **保存为测试样例**（开发者模式）：把文字已替换的网页副本保存为你电脑上的文件。

扩展的内容安全策略只允许它连接自身的文件，每次自动化测试都会检查它没有发出任何网络请求。

## rex 不做什么

不出售、分享或转让数据；不把数据用于广告、信用评估或除向你展示所请求网页的记录之外的任何用途；
不运行远程代码（配方和 WebAssembly 引擎都在扩展包内）。

## 儿童

rex 是通用工具，不会有意处理儿童的数据。

## 变更与联系

本政策的变更会发布在此文件中，并更新生效日期：<https://github.com/Shuozeli/rex/blob/main/PRIVACY.md>。
有问题请在 <https://github.com/Shuozeli/rex/issues> 提交 issue。
