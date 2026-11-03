const http = require('http');
const fs = require('fs');
const path = require('path');
const url = require('url');

const PORT = 3000;

// MIME types
const mimeTypes = {
  '.html': 'text/html',
  '.css': 'text/css',
  '.js': 'application/javascript',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
  '.json': 'application/json',
  '.ico': 'image/x-icon'
};

const server = http.createServer((req, res) => {
  const parsedUrl = url.parse(req.url);
  let filePath = path.join(__dirname, 'src', 'renderer', parsedUrl.pathname);
  
  // If no path, serve index.html
  if (parsedUrl.pathname === '/') {
    filePath = path.join(__dirname, 'src', 'renderer', 'index.html');
  }
  
  // Add extension if not present
  if (!path.extname(filePath)) {
    filePath += '.html';
  }
  
  // Check if file exists
  if (fs.existsSync(filePath)) {
    const ext = path.extname(filePath);
    const contentType = mimeTypes[ext] || 'application/octet-stream';
    
    fs.readFile(filePath, (err, content) => {
      if (err) {
        res.writeHead(500);
        res.end('Error loading file');
      } else {
        res.writeHead(200, { 'Content-Type': contentType });
        res.end(content);
      }
    });
  } else {
    res.writeHead(404);
    res.end('File not found');
  }
});

server.listen(PORT, () => {
  console.log(`Tauri dev server running at http://localhost:${PORT}`);
});