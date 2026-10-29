// This page is external to the desktop. The desktop UI remains the upstream UI.
export const devicePage = `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Connect Cryzo</title>
<style>body{font:16px system-ui;background:#111;color:#eee;max-width:440px;margin:12vh auto;padding:24px}button,input{font:inherit;padding:12px;border-radius:8px;margin:8px 4px 8px 0}button{cursor:pointer}code{font-size:24px}p{line-height:1.6}#message{color:#ccc}</style>
<h1>Connect Cryzo Free</h1><p>Connect your desktop to the free NVIDIA models. Your account includes 100 model calls each day.</p>
<p id="message" role="status">Checking sign-in…</p><button id="login" hidden>Sign in with Google</button>
<section id="approval" hidden><p>Confirm this code matches the code displayed in your Cryzo desktop:</p><p><code id="code"></code></p><p>This authorizes the Cryzo desktop to use model inference with your account. Only approve a request you started on a device in your possession.</p><button id="approve">Connect desktop</button><button id="deny">Cancel</button></section>
<script>
const code=new URLSearchParams(location.search).get('user_code')||'';
const message=document.getElementById('message');
async function request(path,body){const response=await fetch('/api/auth/'+path,{method:body?'POST':'GET',headers:body?{'content-type':'application/json'}:{},body:body?JSON.stringify(body):undefined});const data=await response.json();if(!response.ok)throw new Error(data.message||'Could not complete sign-in');return data}
async function load(){if(!/^[A-Z0-9-]{4,32}$/i.test(code))throw new Error('Open the sign-in link from Cryzo desktop.');const session=await request('get-session');if(!session?.user){message.textContent='Sign in to connect your desktop.';document.getElementById('login').hidden=false;return}await request('device?user_code='+encodeURIComponent(code));document.getElementById('code').textContent=code;document.getElementById('approval').hidden=false;message.textContent='Signed in. Confirm your device below.'}
document.getElementById('login').onclick=async()=>{try{const data=await request('sign-in/social',{provider:'google',callbackURL:location.href});if(data.url)location.assign(data.url)}catch(error){message.textContent=error.message}};
for(const action of ['approve','deny'])document.getElementById(action).onclick=async()=>{try{await request('device/'+action,{userCode:code});document.getElementById('approval').hidden=true;message.textContent=action==='approve'?'Connected. Return to Cryzo desktop.':'Connection cancelled.'}catch(error){message.textContent=error.message}};
load().catch(error=>message.textContent=error.message);
</script></html>`
