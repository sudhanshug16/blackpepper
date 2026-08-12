"use strict";

const tabs = Array.from(document.querySelectorAll('[role="tab"]'));

function selectTab(tab, moveFocus) {
  for (const candidate of tabs) {
    const selected = candidate === tab;
    candidate.setAttribute("aria-selected", String(selected));
    candidate.tabIndex = selected ? 0 : -1;

    const panel = document.getElementById(candidate.getAttribute("aria-controls"));
    if (panel) panel.hidden = !selected;
  }

  if (moveFocus) tab.focus();
}

tabs.forEach((tab, index) => {
  tab.addEventListener("click", () => selectTab(tab, false));
  tab.addEventListener("keydown", (event) => {
    let next = index;
    if (event.key === "ArrowRight") next = (index + 1) % tabs.length;
    else if (event.key === "ArrowLeft") next = (index - 1 + tabs.length) % tabs.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = tabs.length - 1;
    else return;

    event.preventDefault();
    selectTab(tabs[next], true);
  });
});

const copyButton = document.getElementById("copy-install");
const installCode = document.getElementById("install-code");
const copyStatus = document.getElementById("copy-status");
let statusTimer;

async function copyInstallCommand() {
  const command = installCode.textContent.trim();
  let copied = false;

  try {
    await navigator.clipboard.writeText(command);
    copied = true;
  } catch (_) {
    const input = document.createElement("textarea");
    input.value = command;
    input.setAttribute("readonly", "");
    input.style.position = "fixed";
    input.style.left = "-9999px";
    document.body.appendChild(input);
    input.select();
    copied = document.execCommand("copy");
    input.remove();
  }

  copyButton.textContent = copied ? "copied" : "select command";
  copyStatus.textContent = copied
    ? "install command copied"
    : "copy was blocked; select the command manually";

  window.clearTimeout(statusTimer);
  statusTimer = window.setTimeout(() => {
    copyButton.textContent = "copy";
    copyStatus.textContent = "checksum-verifies the release bundle";
  }, 2400);
}

copyButton.addEventListener("click", copyInstallCommand);
