self.onmessage = function (e) {
  postMessage(e.data * 2);
};
