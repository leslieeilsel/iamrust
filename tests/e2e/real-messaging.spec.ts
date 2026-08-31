import { expect, test, type Page } from "@playwright/test";

interface TestUser {
  email: string;
  username: string;
  nickname: string;
  password: string;
}

test("two real accounts complete friendship, direct chat, group chat, and offline sync", async ({
  browser,
}) => {
  const suffix = crypto.randomUUID().replaceAll("-", "").slice(0, 12);
  const alice: TestUser = {
    email: `alice-${suffix}@example.test`,
    username: `alice_${suffix}`,
    nickname: `Alice ${suffix.slice(0, 4)}`,
    password: "SecurePass123",
  };
  const bob: TestUser = {
    email: `bob-${suffix}@example.test`,
    username: `bob_${suffix}`,
    nickname: `Bob ${suffix.slice(0, 4)}`,
    password: "SecurePass123",
  };
  const aliceContext = await browser.newContext();
  const bobContext = await browser.newContext();
  try {
    const alicePage = await aliceContext.newPage();
    const bobPage = await bobContext.newPage();
    await Promise.all([register(alicePage, alice), register(bobPage, bob)]);

    await alicePage.getByRole("button", { name: "联系人" }).click();
    await alicePage.getByRole("button", { name: "添加好友" }).click();
    const addFriend = alicePage.getByRole("dialog", { name: "添加好友" });
    await addFriend.getByPlaceholder("输入完整用户名").fill(bob.username);
    await addFriend.getByRole("button", { name: "搜索" }).click();
    await expect(addFriend.getByText(`@${bob.username}`)).toBeVisible();
    await addFriend.getByRole("button", { name: "申请好友" }).click();
    await expect(addFriend.getByText(/已发送|申请处理中/u)).toBeVisible();
    await addFriend.getByRole("button", { name: "关闭" }).click();

    await bobPage.getByRole("button", { name: "联系人" }).click();
    await bobPage
      .getByRole("button", {
        name: new RegExp(`接受 ${escapeRegExp(alice.nickname)}`),
      })
      .click();
    await expect(
      bobPage.getByText(alice.nickname, { exact: true }).first(),
    ).toBeVisible();

    await openConversation(alicePage, bob.nickname);
    await sendMessage(alicePage, "hello from Alice");
    await openConversation(bobPage, alice.nickname);
    await expect(
      messageLog(bobPage).getByText("hello from Alice", { exact: true }),
    ).toBeVisible();
    await sendMessage(bobPage, "hello from Bob");
    await expect(
      messageLog(alicePage).getByText("hello from Bob", { exact: true }),
    ).toBeVisible();

    await alicePage.getByRole("button", { name: "创建群聊" }).click();
    const createGroup = alicePage.getByRole("dialog", { name: "创建群聊" });
    const groupName = `Rust team ${suffix.slice(0, 4)}`;
    await createGroup.getByLabel("群名称").fill(groupName);
    await createGroup
      .getByRole("group", { name: "选择群成员" })
      .getByRole("button", { name: new RegExp(escapeRegExp(bob.nickname)) })
      .click();
    await createGroup
      .getByRole("button", { name: "创建", exact: true })
      .click();
    await expect(
      alicePage.getByRole("heading", { name: groupName }),
    ).toBeVisible();
    await sendMessage(alicePage, "hello group");

    await openConversation(bobPage, groupName);
    await expect(
      messageLog(bobPage).getByText("hello group", { exact: true }),
    ).toBeVisible();

    const imageChooser = bobPage.waitForEvent("filechooser");
    await bobPage.getByRole("button", { name: "选择图片" }).click();
    await (
      await imageChooser
    ).setFiles({
      name: "pixel.gif",
      mimeType: "image/gif",
      buffer: Buffer.from(
        "R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==",
        "base64",
      ),
    });
    await expect(
      bobPage.getByLabel("待发送附件").getByText("pixel.gif"),
    ).toBeVisible();
    await bobPage.getByRole("button", { name: "发送消息" }).click();
    await expect(
      messageLog(alicePage).getByRole("button", { name: "查看图片 pixel.gif" }),
    ).toBeVisible({ timeout: 10_000 });

    const fileChooser = bobPage.waitForEvent("filechooser");
    await bobPage.getByRole("button", { name: "选择文件" }).click();
    await (
      await fileChooser
    ).setFiles({
      name: "notes.txt",
      mimeType: "text/plain",
      buffer: Buffer.from("I Am Rust E2E attachment\n"),
    });
    await expect(
      bobPage.getByLabel("待发送附件").getByText("notes.txt"),
    ).toBeVisible();
    await bobPage.getByRole("button", { name: "发送消息" }).click();
    await expect(
      messageLog(alicePage).getByText("notes.txt", { exact: true }),
    ).toBeVisible({
      timeout: 10_000,
    });

    await bobContext.setOffline(true);
    await expect(bobPage.getByText("当前离线，消息将在恢复连接后发送")).toBeVisible();
    await sendMessage(alicePage, "delivered after reconnect");
    await bobContext.setOffline(false);
    await expect(
      messageLog(bobPage).getByText("delivered after reconnect", {
        exact: true,
      }),
    ).toBeVisible({ timeout: 15_000 });
    await expect(
      messageLog(bobPage).getByText("delivered after reconnect", {
        exact: true,
      }),
    ).toHaveCount(1);
  } finally {
    await Promise.all([aliceContext.close(), bobContext.close()]);
  }
});

async function register(page: Page, user: TestUser): Promise<void> {
  await page.goto("http://127.0.0.1:1421");
  await page.getByRole("tab", { name: "注册" }).click();
  await page.getByLabel("邮箱").fill(user.email);
  await page.getByLabel("用户名").fill(user.username);
  await page.getByLabel("昵称").fill(user.nickname);
  await page.locator('input[name="password"]').fill(user.password);
  await page.getByRole("button", { name: "注册并登录" }).click();
  await expect(
    page.getByRole("navigation", { name: "主要导航" }),
  ).toBeVisible();
}

async function openConversation(page: Page, name: string): Promise<void> {
  await page.getByRole("button", { name: "会话", exact: true }).click();
  const row = page.locator(".conversation-row").filter({ hasText: name });
  await expect(row).toBeVisible({ timeout: 10_000 });
  await row.click();
  await expect(page.locator(".chat-header h2")).toHaveText(name);
}

async function sendMessage(page: Page, text: string): Promise<void> {
  const composer = page.getByRole("textbox", { name: "输入消息" });
  await composer.fill(text);
  await composer.press("Enter");
  await expect(messageLog(page).getByText(text, { exact: true })).toBeVisible();
}

function messageLog(page: Page) {
  return page.getByLabel("消息记录");
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}
