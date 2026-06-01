'use strict';

// CommonJS stand-in for @angular/core (the CJS jest project requires it).

function Component(meta) {
  return function (target) {
    target.__annotations__ = (target.__annotations__ || []).concat([meta]);
    return target;
  };
}

function Input() {
  return function () {};
}

function Output() {
  return function () {};
}

module.exports = { Component, Input, Output };
