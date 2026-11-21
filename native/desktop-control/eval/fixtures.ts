// The local web pages the browser tasks use, and the form posts they receive.

export function startFixtures() {
  const received: Record<string, string>[] = []
  const page = (title: string, body: string) =>
    new Response(`<!doctype html><html><head><meta charset="utf-8"><title>${title}</title></head><body style="font-family:sans-serif;margin:40px">${body}</body></html>`, {
      headers: { "content-type": "text/html; charset=utf-8" },
    })
  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    async fetch(request) {
      const url = new URL(request.url)
      if (url.pathname === "/prices") {
        const rows = [
          ["Desk Chair", "$129.00"],
          ["Blue Lamp", "$34.75"],
          ["Red Lamp", "$36.20"],
          ["Bookshelf", "$89.99"],
          ["Blue Rug", "$64.10"],
          ["Lamp Shade", "$12.40"],
        ]
        return page("Prices", `<h1>Store prices</h1><table border="1" cellpadding="6"><tr><th>Item</th><th>Price</th></tr>${rows.map(([item, price]) => `<tr><td>${item}</td><td>${price}</td></tr>`).join("")}</table>`)
      }
      if (url.pathname === "/form")
        return page(
          "Sign up",
          `<h1>Sign up</h1><form method="post" action="/submit">
            <p><label>Full name <input name="name"></label></p>
            <p><label>Email <input name="email" type="email"></label></p>
            <p><label>Plan <select name="plan"><option value="">Choose…</option><option value="free">Free</option><option value="pro">Pro</option><option value="team">Team</option></select></label></p>
            <p><label><input type="checkbox" name="terms"> I accept the terms</label></p>
            <p><button type="submit">Create account</button></p></form>`,
        )
      if (url.pathname === "/submit" && request.method === "POST") {
        received.push(Object.fromEntries(new URLSearchParams(await request.text())))
        return page("Thanks", "<h1>Account created</h1><p>Thanks for signing up.</p>")
      }
      if (url.pathname === "/files")
        return page("Files", `<h1>Files</h1><p><a href="/report.csv" download="cryzo-eval-report.csv">Download report</a></p><p><a href="/prices">Other page</a></p>`)
      if (url.pathname === "/report.csv")
        return new Response("item,amount\nlamps,2\nrugs,1\n", {
          headers: { "content-type": "text/csv", "content-disposition": 'attachment; filename="cryzo-eval-report.csv"' },
        })
      if (url.pathname === "/code") return page("Confirmation", "<h1>Order placed</h1><p>Your confirmation code is <strong>QX-7341</strong>.</p>")
      return new Response("not found", { status: 404 })
    },
  })
  return { server, url: `http://127.0.0.1:${server.port}`, received }
}
