import 'package:flutter/rendering.dart';

import 'command_decoder.dart';
import 'nve_canvas_controller.dart';
import 'package:nve_host/nve_host.dart';
import 'package:nve_host/nve_host_ffi.dart';
import 'scene_canvas_store.dart';

/// Shared live Flutter [SemanticsNode] assembler for [RenderNveCanvas], supporting both
/// direct `NveSceneView` slabs ([NveSceneCanvasStore]) and retained [NveRetainedTree] mode.
class NveCanvasSemanticsAssembler {
  static const double _inferredHeadingFontSize = 20.0;

  final Map<int, SemanticsNode> _nodes = <int, SemanticsNode>{};

  void clear() => _nodes.clear();

  /// Rebuilds/updates child [SemanticsNode]s in place (preserving stable node IDs across
  /// frames) and returns them in ascending paint order.
  List<SemanticsNode> buildSceneSemantics({
    required NveSceneCanvasStore store,
    required Rect surfaceRect,
    required void Function(int nodeId, Offset center) onTapNode,
  }) {
    final built = <SemanticsNode>[];
    final activeIds = <int>{};
    final rootId = store.rootNodeId;
    if (rootId != null) {
      _collectScene(store, rootId, Offset.zero, surfaceRect, activeIds, built, onTapNode);
    }
    _nodes.removeWhere((int id, SemanticsNode _) => !activeIds.contains(id));
    return built;
  }

  /// Rebuilds/updates child [SemanticsNode]s for retained-tree mode.
  List<SemanticsNode> buildRetainedSemantics({
    required NveCanvasController controller,
    required Rect surfaceRect,
    required void Function(int nodeId, Offset center) onTapNode,
  }) {
    final built = <SemanticsNode>[];
    final activeIds = <int>{};
    _collectRetained(controller, nveRootNodeId, Offset.zero, surfaceRect, activeIds, built, onTapNode);
    _nodes.removeWhere((int id, SemanticsNode _) => !activeIds.contains(id));
    return built;
  }

  void _collectScene(
    NveSceneCanvasStore store,
    int nodeId,
    Offset scrollTranslate,
    Rect clipRect,
    Set<int> activeIds,
    List<SemanticsNode> out,
    void Function(int nodeId, Offset center) onTapNode,
  ) {
    final n = store.node(nodeId);
    if (n == null || n.opacityU8 == 0) return;

    final rect = n.rect.shift(scrollTranslate);
    final visibleRect = rect.intersect(clipRect);
    final isVisible = !visibleRect.isEmpty && rect.width > 0.0 && rect.height > 0.0;
    final meta = store.metadataOf(nodeId);

    final role = meta?.accessibilityRole ?? _inferSceneRole(n);
    final isButton = role == NveAccessibilityRole.button ||
        n.kind == nveNodeKindPressable ||
        (n.isInteractive && !n.isTextInput && !n.isScrollView);
    final isTextField = role == NveAccessibilityRole.textField || n.isTextInput;
    final isHeader = role == NveAccessibilityRole.header;
    final isImage = role == NveAccessibilityRole.image || n.isImage;
    final isDisabled = meta?.disabled ?? false;

    String label = meta?.accessibilityLabel ?? '';
    String value = '';
    String hint = meta?.accessibilityHint ?? '';

    if (isTextField) {
      final placeholder = meta?.placeholder ?? '';
      if (label.isEmpty && placeholder.isNotEmpty) {
        label = placeholder;
      }
      if (n.text.isNotEmpty) {
        value = (meta?.secureTextEntry ?? false)
            ? List<String>.filled(n.text.runes.length, '\u2022').join()
            : n.text;
      }
      if (hint.isEmpty && placeholder.isNotEmpty && label != placeholder) {
        hint = placeholder;
      }
    } else if (isButton) {
      if (label.isEmpty) {
        label = store.collectTextLabel(nodeId);
      }
    } else if (n.isLeafText) {
      if (label.isEmpty) {
        label = n.text;
      }
    }

    final shouldExpose = isVisible &&
        (isButton ||
            isTextField ||
            isHeader ||
            (isImage && label.isNotEmpty) ||
            (n.isLeafText && label.isNotEmpty) ||
            (meta?.accessibilityLabel?.isNotEmpty ?? false));

    if (shouldExpose) {
      final childConfig = SemanticsConfiguration()
        ..textDirection = TextDirection.ltr
        ..label = label
        ..value = value
        ..hint = hint
        ..isButton = isButton
        ..isTextField = isTextField
        ..isHeader = isHeader
        ..isImage = isImage
        ..isObscured = isTextField && (meta?.secureTextEntry ?? false);

      if (isButton || isTextField) {
        childConfig.isEnabled = !isDisabled;
        if (!isDisabled) {
          final tapPoint = rect.center;
          childConfig.onTap = () => onTapNode(nodeId, tapPoint);
        }
      }

      final semNode = _nodes.putIfAbsent(nodeId, SemanticsNode.new)
        ..rect = visibleRect
        ..updateWith(config: childConfig);
      activeIds.add(nodeId);
      out.add(semNode);

      if (isButton && n.children.isNotEmpty && meta?.accessibilityLabel == null) {
        return;
      }
    }

    if (n.children.isNotEmpty) {
      final nextClip = n.clipsBounds && !visibleRect.isEmpty ? visibleRect : clipRect;
      final nextTranslate = n.isScrollView
          ? scrollTranslate - store.scrollOffsetOf(nodeId)
          : scrollTranslate;
      final ordered = store.childrenInPaintOrder(n.children);
      for (final childId in ordered) {
        _collectScene(store, childId, nextTranslate, nextClip, activeIds, out, onTapNode);
      }
    }
  }

  NveAccessibilityRole _inferSceneRole(NveSceneNodeSnapshot n) {
    if (n.kind == nveNodeKindTextInput) return NveAccessibilityRole.textField;
    if (n.kind == nveNodeKindPressable) return NveAccessibilityRole.button;
    if (n.kind == nveNodeKindImage) return NveAccessibilityRole.image;
    if (n.kind == nveNodeKindScrollView) return NveAccessibilityRole.scrollView;
    if (n.kind == nveNodeKindText && n.isLeafText && n.fontSize >= _inferredHeadingFontSize) {
      return NveAccessibilityRole.header;
    }
    if (n.isLeafText) return NveAccessibilityRole.text;
    return NveAccessibilityRole.none;
  }

  void _collectRetained(
    NveCanvasController ctrl,
    int nodeId,
    Offset scrollTranslate,
    Rect clipRect,
    Set<int> activeIds,
    List<SemanticsNode> out,
    void Function(int nodeId, Offset center) onTapNode,
  ) {
    final tree = ctrl.tree;
    final node = tree.node(nodeId);
    if (node == null || !node.hasComputedLayout || node.style.opacity <= 0.001) return;

    final rect = node.computedRect.shift(scrollTranslate);
    final visibleRect = rect.intersect(clipRect);
    final isVisible = !visibleRect.isEmpty && rect.width > 0.0 && rect.height > 0.0;

    final roleRaw = (node.rawProps['accessibilityRole']?.asString ??
            node.rawProps['role']?.asString ??
            '')
        .trim()
        .toLowerCase();
    final isHeader = roleRaw == 'heading' ||
        roleRaw == 'header' ||
        (roleRaw.isEmpty && node.isTextLeaf && node.style.fontSize >= _inferredHeadingFontSize);
    final isTextField = roleRaw == 'textfield' ||
        roleRaw == 'textbox' ||
        node.kind == NveComponentKind.textInput;
    final isButton = roleRaw == 'button' ||
        node.kind == NveComponentKind.pressable ||
        (node.isInteractive && !isTextField && node.kind != NveComponentKind.scrollView);
    final isImage = roleRaw == 'image' || node.kind == NveComponentKind.image;
    final isSecure = node.rawProps['secureTextEntry']?.asBool ?? false;
    final isDisabled = (node.rawProps['accessibilityDisabled']?.asBool ??
        node.rawProps['disabled']?.asBool ??
        false);

    var label = node.rawProps['accessibilityLabel']?.asString ??
        node.rawProps['aria-label']?.asString ??
        '';
    var value = '';
    var hint = node.rawProps['accessibilityHint']?.asString ?? '';

    if (isTextField) {
      final placeholder = node.rawProps['placeholder']?.asString ?? '';
      final rawVal = node.rawProps['value']?.asString ?? node.text ?? '';
      if (label.isEmpty && placeholder.isNotEmpty) label = placeholder;
      if (rawVal.isNotEmpty) {
        value = isSecure ? List<String>.filled(rawVal.runes.length, '\u2022').join() : rawVal;
      }
      if (hint.isEmpty && placeholder.isNotEmpty && label != placeholder) hint = placeholder;
    } else if (isButton && label.isEmpty) {
      label = _collectRetainedLabel(tree, nodeId);
    } else if (node.isTextLeaf && label.isEmpty) {
      label = node.text ?? '';
    }

    final shouldExpose = isVisible &&
        (isButton ||
            isTextField ||
            isHeader ||
            (isImage && label.isNotEmpty) ||
            (node.isTextLeaf && label.isNotEmpty) ||
            label.isNotEmpty);

    if (shouldExpose) {
      final childConfig = SemanticsConfiguration()
        ..textDirection = TextDirection.ltr
        ..label = label
        ..value = value
        ..hint = hint
        ..isButton = isButton
        ..isTextField = isTextField
        ..isHeader = isHeader
        ..isImage = isImage
        ..isObscured = isTextField && isSecure;

      if (isButton || isTextField) {
        childConfig.isEnabled = !isDisabled;
        if (!isDisabled) {
          final tapPoint = rect.center;
          childConfig.onTap = () => onTapNode(nodeId, tapPoint);
        }
      }

      final semNode = _nodes.putIfAbsent(nodeId, SemanticsNode.new)
        ..rect = visibleRect
        ..updateWith(config: childConfig);
      activeIds.add(nodeId);
      out.add(semNode);

      if (isButton &&
          node.children.isNotEmpty &&
          node.rawProps['accessibilityLabel'] == null &&
          node.rawProps['aria-label'] == null) {
        return;
      }
    }

    final isScrollView = node.kind == NveComponentKind.scrollView;
    final nextClip = isScrollView && !visibleRect.isEmpty ? visibleRect : clipRect;
    final nextTranslate = isScrollView
        ? scrollTranslate - ctrl.scrollOffsetOf(nodeId)
        : scrollTranslate;
    final ordered = childrenInPaintOrder(tree, node.children);
    for (final childId in ordered) {
      _collectRetained(ctrl, childId, nextTranslate, nextClip, activeIds, out, onTapNode);
    }
  }

  String _collectRetainedLabel(NveRetainedTree tree, int nodeId) {
    final node = tree.node(nodeId);
    if (node == null) return '';
    if (node.isTextLeaf && (node.text?.isNotEmpty ?? false)) return node.text!;
    final parts = <String>[];
    for (final cid in node.children) {
      final sub = _collectRetainedLabel(tree, cid);
      if (sub.isNotEmpty) parts.add(sub);
    }
    return parts.join(' ');
  }
}
