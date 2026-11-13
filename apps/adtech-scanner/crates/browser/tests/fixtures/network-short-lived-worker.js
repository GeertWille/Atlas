importScripts('/network-short-lived.js');
fetchAsAsked(new URLSearchParams(self.location.search)).then(function () {
  postMessage('read');
  close();
});
