import init, { run } from "./pkg/papo_browser_worker.js";

const ready = init();

self.onconnect = ({ ports: [port] }) => {
  port.onmessage = async ({ data }) => {
    await ready;
    run(port, data.url, data.nick);
  };
  port.start();
};
