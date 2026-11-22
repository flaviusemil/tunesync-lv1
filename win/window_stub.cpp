// window_stub.cpp - a Window that does nothing, for builds without the window (the
// first cross-build check). The real one is window.cpp.
#include "window.h"
namespace trw {
struct Window::Impl {};
Window* Window::start(App*) { return nullptr; }
void Window::show() {}
void Window::stop(unsigned) {}
}  // namespace trw
