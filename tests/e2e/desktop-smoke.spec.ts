import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test("auth and core desktop flow are keyboard accessible", async ({ page }) => {
  await page.goto("/");
  await expect(
    page.getByRole("tab", { name: "登录", exact: true }),
  ).toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

  await page.getByRole("button", { name: "进入本地演示" }).click();
  await expect(page.getByRole("main")).toHaveClass(/app-frame/u);
  await expect(page.getByRole("textbox", { name: "输入消息" })).toBeVisible();

  const composer = page.getByRole("textbox", { name: "输入消息" });
  await composer.fill("E2E 🦀 message");
  await composer.press("Enter");
  await expect(
    page.getByLabel("消息记录").getByText("E2E 🦀 message", { exact: true }),
  ).toBeVisible();
  await expect(composer).toHaveValue("");

  await page.keyboard.press(
    process.platform === "darwin" ? "Meta+," : "Control+,",
  );
  await expect(page.getByRole("heading", { name: "设置" })).toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

  await page.getByRole("button", { name: "外观" }).click();
  await page.getByRole("radio", { name: "English" }).check();
  await expect(page.locator("html")).toHaveAttribute("lang", "en-US");
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  await expect(
    page.getByRole("navigation", { name: "Main navigation" }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Contacts" })).toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});

test("minimum window and 200 percent zoom keep primary actions available", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByRole("button", { name: "进入本地演示" }).click();
  await page.evaluate(() => {
    document.documentElement.style.zoom = "2";
  });
  await expect(
    page.getByRole("navigation", { name: "主要导航" }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "发送消息" })).toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
