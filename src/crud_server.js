"use strict";
const http = require("node:http");
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");

const root = __dirname;
const dataFile = path.join(root, "records.json");
const port = Number(process.env.PORT || 3000);
const fields = [__RUX_FIELDS__];
const limit = 1024 * 1024;
let records = [];
try {
  const parsed = JSON.parse(fs.readFileSync(dataFile, "utf8"));
  if (!Array.isArray(parsed)) throw new Error("expected array");
  records = parsed;
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
function persist(next) {
  const temporary = dataFile + "." + process.pid + ".tmp";
  fs.writeFileSync(temporary, JSON.stringify(next, null, 2), { mode: 0o600 });
  fs.renameSync(temporary, dataFile);
  records = next;
}
function respond(res, status, data) {
  const payload = JSON.stringify(data);
  res.writeHead(status, { "Content-Type": "application/json; charset=utf-8", "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff" });
  res.end(payload);
}
function readBody(req) {
  return new Promise((resolve, reject) => {
    let raw = "";
    req.on("data", chunk => {
      raw += chunk;
      if (raw.length > limit) { reject(new Error("Request too large")); req.destroy(); }
    });
    req.on("end", () => { try { resolve(JSON.parse(raw)); } catch { reject(new Error("Invalid JSON")); } });
    req.on("error", reject);
  });
}
function validate(input) {
  if (!input || typeof input !== "object" || Array.isArray(input)) return null;
  const values = {};
  for (const field of fields) {
    if (typeof input[field] !== "string" || !input[field].trim() || input[field].length > 250) return null;
    values[field] = input[field].trim();
  }
  return values;
}
const mime = { ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".json": "application/json; charset=utf-8", ".css": "text/css; charset=utf-8" };
const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, "http://localhost");
    if (url.pathname === "/api/records") {
      if (req.method === "GET") return respond(res, 200, records);
      if (req.method === "POST") {
        const values = validate(await readBody(req));
        if (!values) return respond(res, 400, { error: "Invalid fields" });
        const item = { id: crypto.randomUUID(), ...values };
        persist([...records, item]);
        return respond(res, 201, item);
      }
      return respond(res, 405, { error: "Method not allowed" });
    }
    const match = /^\/api\/records\/([a-f0-9-]{36})$/.exec(url.pathname);
    if (match) {
      const id = match[1];
      const index = records.findIndex(item => item.id === id);
      if (index < 0) return respond(res, 404, { error: "Not found" });
      if (req.method === "PUT") {
        const values = validate(await readBody(req));
        if (!values) return respond(res, 400, { error: "Invalid fields" });
        const item = { id, ...values };
        persist(records.map(record => record.id === id ? item : record));
        return respond(res, 200, item);
      }
      if (req.method === "DELETE") {
        persist(records.filter(record => record.id !== id));
        return respond(res, 200, { deleted: true });
      }
      return respond(res, 405, { error: "Method not allowed" });
    }
    if (req.method !== "GET" && req.method !== "HEAD") return respond(res, 405, { error: "Method not allowed" });
    const files = { "/": "index.html", "/index.html": "index.html", "/manifest.webmanifest": "manifest.webmanifest" };
    const filename = files[url.pathname];
    if (!filename) return respond(res, 404, { error: "Not found" });
    const file = path.join(root, filename);
    if (!fs.existsSync(file)) return respond(res, 404, { error: "Not found" });
    res.writeHead(200, { "Content-Type": mime[path.extname(file)] || "application/manifest+json", "X-Content-Type-Options": "nosniff" });
    if (req.method === "HEAD") return res.end();
    fs.createReadStream(file).pipe(res);
  } catch (error) {
    console.error(error.message);
    if (!res.headersSent) respond(res, 500, { error: "Internal server error" });
  }
});
server.listen(port, "127.0.0.1", () => console.log("RUX CRUD: http://localhost:" + port));
