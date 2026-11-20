#!/usr/bin/env python3
"""Make the PowerPoint fixtures the import tests read (tests/pptx.rs).

    python3 gen_pptx.py OUT_DIR

Needs python-pptx (1.0.2) and Pillow. Writes deck16x9.pptx, deck4x3.pptx and
extras.pptx to OUT_DIR; copy them next to this script. Run it from a scratch directory,
not from here, so nothing else lands in the fixtures folder.
"""
import copy
import io
import sys
from pathlib import Path

from lxml import etree
from PIL import Image, ImageDraw
from pptx import Presentation
from pptx.chart.data import CategoryChartData
from pptx.dml.color import RGBColor
from pptx.enum.chart import XL_CHART_TYPE
from pptx.enum.dml import MSO_THEME_COLOR
from pptx.enum.shapes import MSO_CONNECTOR, MSO_SHAPE
from pptx.enum.text import PP_ALIGN
from pptx.oxml.ns import qn
from pptx.util import Emu, Inches, Pt

MC = "http://schemas.openxmlformats.org/markup-compatibility/2006"
P = "http://schemas.openxmlformats.org/presentationml/2006/main"
A = "http://schemas.openxmlformats.org/drawingml/2006/main"
CX1 = "http://schemas.microsoft.com/office/drawing/2015/9/8/chartex"


def png(w, h, colour):
    img = Image.new("RGB", (w, h), colour)
    d = ImageDraw.Draw(img)
    d.ellipse((w // 4, h // 4, 3 * w // 4, 3 * h // 4), fill=(255, 255, 255))
    out = io.BytesIO()
    img.save(out, "PNG")
    out.seek(0)
    return out


def notes(slide, text):
    slide.notes_slide.notes_text_frame.text = text


def blank(prs):
    return prs.slides.add_slide(prs.slide_layouts[6])


def deck16x9(out):
    prs = Presentation()
    # The template is 4:3: widen its masters and layouts with the slides,
    # as PowerPoint does when the slide size changes.
    sx = Inches(13.333) / prs.slide_width
    prs.slide_width = Inches(13.333)
    prs.slide_height = Inches(7.5)
    for master in prs.slide_masters:
        for part in [master, *master.slide_layouts]:
            for sh in part.shapes:
                # Only shapes placed there; the rest take their place from the master.
                xfrm = sh._element.find(qn("p:spPr") + "/" + qn("a:xfrm"))
                if xfrm is not None and xfrm.find(qn("a:off")) is not None:
                    sh.left, sh.width = Emu(round(sh.left * sx)), Emu(round(sh.width * sx))

    # 1. Title slide: title and subtitle placeholders, geometry from the layout.
    s = prs.slides.add_slide(prs.slide_layouts[0])
    s.shapes.title.text = "Zebra Import Test"
    s.placeholders[1].text = "A deck made by python-pptx"
    notes(s, "Opening notes")

    # 2. Title and content: bullets at three levels.
    s = prs.slides.add_slide(prs.slide_layouts[1])
    s.shapes.title.text = "Bullets"
    tf = s.placeholders[1].text_frame
    tf.text = "Level one"
    for text, level in [("Level two", 1), ("Level three", 2), ("Back to one", 0)]:
        p = tf.add_paragraph()
        p.text = text
        p.level = level
    notes(s, "Talk about bullets")

    # 3. Runs: bold, italic, colour, theme colour, link, underline; sizes.
    s = blank(prs)
    tb = s.shapes.add_textbox(Inches(1), Inches(1), Inches(8), Inches(1.5))
    p = tb.text_frame.paragraphs[0]
    for text, how in [("Bold ", "b"), ("italic ", "i"), ("red ", "red"), ("accent ", "accent"), ("link", "link"), (" under", "u")]:
        r = p.add_run()
        r.text = text
        r.font.size = Pt(24)
        if how == "b":
            r.font.bold = True
        elif how == "i":
            r.font.italic = True
        elif how == "red":
            r.font.color.rgb = RGBColor(0xFF, 0x00, 0x00)
        elif how == "accent":
            r.font.color.theme_color = MSO_THEME_COLOR.ACCENT_1
        elif how == "link":
            r.hyperlink.address = "https://example.com/"
        elif how == "u":
            r.font.underline = True
    p2 = tb.text_frame.add_paragraph()
    p2.text = "Centred line"
    p2.alignment = PP_ALIGN.CENTER
    p2.runs[0].font.size = Pt(24)
    big = s.shapes.add_textbox(Inches(1), Inches(4), Inches(10), Inches(1.5))
    big.text_frame.text = "Big words"
    big.text_frame.paragraphs[0].runs[0].font.size = Pt(54)
    big.text_frame.paragraphs[0].runs[0].font.bold = True
    notes(s, "Runs and sizes")

    # 4. A picture and shapes, some with text.
    s = blank(prs)
    s.shapes.add_picture(png(64, 48, (40, 120, 200)), Inches(0.5), Inches(0.5), Inches(3), Inches(2.25))
    box = s.shapes.add_shape(MSO_SHAPE.RECTANGLE, Inches(4), Inches(0.5), Inches(3), Inches(1.5))
    box.fill.solid()
    box.fill.fore_color.theme_color = MSO_THEME_COLOR.ACCENT_2
    box.line.color.rgb = RGBColor(0x00, 0x00, 0x00)
    box.line.width = Pt(3)
    box.text_frame.text = "Box"
    s.shapes.add_shape(MSO_SHAPE.ROUNDED_RECTANGLE, Inches(8), Inches(0.5), Inches(3), Inches(1.5))
    oval = s.shapes.add_shape(MSO_SHAPE.OVAL, Inches(0.5), Inches(3.5), Inches(2), Inches(2))
    oval.fill.solid()
    oval.fill.fore_color.rgb = RGBColor(0x22, 0xAA, 0x44)
    s.shapes.add_shape(MSO_SHAPE.ISOSCELES_TRIANGLE, Inches(3), Inches(3.5), Inches(2), Inches(2))
    s.shapes.add_shape(MSO_SHAPE.RIGHT_ARROW, Inches(5.5), Inches(3.5), Inches(2.5), Inches(1.2))
    line = s.shapes.add_connector(MSO_CONNECTOR.STRAIGHT, Inches(8.5), Inches(3.5), Inches(11.5), Inches(5.5))
    line.line.width = Pt(2)
    line.line.color.rgb = RGBColor(0x33, 0x33, 0x33)
    ln = line.line._get_or_add_ln()
    tail = etree.SubElement(ln, qn("a:tailEnd"))
    tail.set("type", "triangle")
    s.shapes.add_shape(MSO_SHAPE.GEAR_6, Inches(9), Inches(5.8), Inches(1.4), Inches(1.4))
    notes(s, "Pictures and shapes")

    # 5. A rotated group, a rotated shape and a flipped triangle.
    s = blank(prs)
    g = s.shapes.add_group_shape()
    g.shapes.add_shape(MSO_SHAPE.RECTANGLE, Inches(1), Inches(1.5), Inches(3), Inches(2))
    g.shapes.add_shape(MSO_SHAPE.OVAL, Inches(4.5), Inches(1.5), Inches(2), Inches(2))
    t = g.shapes.add_textbox(Inches(1), Inches(3.7), Inches(5), Inches(0.8))
    t.text_frame.text = "Grouped"
    g.rotation = 30
    r = s.shapes.add_shape(MSO_SHAPE.RECTANGLE, Inches(8), Inches(1), Inches(3), Inches(1))
    r.rotation = 45
    tri = s.shapes.add_shape(MSO_SHAPE.ISOSCELES_TRIANGLE, Inches(8), Inches(4), Inches(2), Inches(2))
    tri._element.spPr.get_or_add_xfrm().set("flipV", "1")

    # 6. Hidden.
    s = prs.slides.add_slide(prs.slide_layouts[5])
    s.shapes.title.text = "Hidden slide"
    s._element.set("show", "0")
    notes(s, "Nobody sees this")

    # 7. A chart (no picture of it in the file) and a table.
    s = prs.slides.add_slide(prs.slide_layouts[5])
    s.shapes.title.text = "Chart"
    data = CategoryChartData()
    data.categories = ["A", "B", "C"]
    data.add_series("Sales", (1, 3, 2))
    s.shapes.add_chart(XL_CHART_TYPE.COLUMN_CLUSTERED, Inches(1), Inches(2), Inches(5), Inches(4), data)
    tbl = s.shapes.add_table(2, 2, Inches(7), Inches(2), Inches(4), Inches(2)).table
    tbl.cell(0, 0).text = "x"

    # 8. A solid background.
    s = prs.slides.add_slide(prs.slide_layouts[5])
    s.shapes.title.text = "Dark background"
    s.background.fill.solid()
    s.background.fill.fore_color.rgb = RGBColor(0x10, 0x20, 0x40)

    prs.save(out)


def deck4x3(out):
    prs = Presentation()  # 10 x 7.5 in
    # 1. A new-style chart PowerPoint saves with a picture of it beside it:
    #    mc:AlternateContent, a cx1 chart as the choice, the picture as the
    #    fallback.
    s = prs.slides.add_slide(prs.slide_layouts[5])
    s.shapes.title.text = "Chart with a picture"
    pic = s.shapes.add_picture(png(80, 60, (200, 80, 40)), Inches(1), Inches(2), Inches(6), Inches(4.5))
    tree = s.shapes._spTree
    ac = etree.SubElement(tree, f"{{{MC}}}AlternateContent", nsmap={"mc": MC})
    choice = etree.SubElement(ac, f"{{{MC}}}Choice", nsmap={"cx1": CX1})
    choice.set("Requires", "cx1")
    gf = copy.deepcopy(tree.find(qn("p:pic")))
    gf.tag = qn("p:graphicFrame")
    for child in list(gf):
        gf.remove(child)
    nv = etree.SubElement(gf, qn("p:nvGraphicFramePr"))
    c = etree.SubElement(nv, qn("p:cNvPr"))
    c.set("id", "50")
    c.set("name", "Chart 1")
    etree.SubElement(nv, qn("p:cNvGraphicFramePr"))
    etree.SubElement(nv, qn("p:nvPr"))
    xfrm = etree.SubElement(gf, qn("p:xfrm"))
    off = etree.SubElement(xfrm, qn("a:off"))
    off.set("x", str(Inches(1)))
    off.set("y", str(Inches(2)))
    ext = etree.SubElement(xfrm, qn("a:ext"))
    ext.set("cx", str(Inches(6)))
    ext.set("cy", str(Inches(4.5)))
    graphic = etree.SubElement(gf, qn("a:graphic"))
    gd = etree.SubElement(graphic, qn("a:graphicData"))
    gd.set("uri", "http://schemas.microsoft.com/office/drawing/2014/chartex")
    choice.append(gf)
    fallback = etree.SubElement(ac, f"{{{MC}}}Fallback")
    fallback.append(pic._element)
    notes(s, "Chart notes")

    # 2. A video: PowerPoint keeps a poster picture for it.
    s = prs.slides.add_slide(prs.slide_layouts[6])
    s.shapes.add_movie(io.BytesIO(b"\x00\x00\x00\x18ftypmp42not a real video"), Inches(1), Inches(1), Inches(4), Inches(3),
                       poster_frame_image=png(40, 30, (10, 10, 10)), mime_type="video/mp4")

    # 3. A plain title and content slide.
    s = prs.slides.add_slide(prs.slide_layouts[1])
    s.shapes.title.text = "Four by three"
    s.placeholders[1].text_frame.text = "One point"
    prs.save(out)


def master_shape(master, sid, name, prst, box, fill):
    """A shape on the slide master (python-pptx adds shapes to slides only)."""
    x, y, w, h = (str(int(v)) for v in box)
    sp = etree.fromstring(
        f'<p:sp xmlns:p="{P}" xmlns:a="{A}"><p:nvSpPr><p:cNvPr id="{sid}" name="{name}"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>'
        f'<p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{h}"/></a:xfrm><a:prstGeom prst="{prst}"><a:avLst/></a:prstGeom>{fill}</p:spPr></p:sp>'
    )
    master.shapes._spTree.append(sp)


def extras(out):
    """What the follow-ups to the first import cover (spec 17b)."""
    prs = Presentation()
    master = prs.slide_master
    # Decorations every slide shows: a shape Zebra doesn't have and a
    # gradient; each is noted once, however many slides show it.
    master_shape(master, 90, "Gear", "gear6", (Inches(9), Inches(6.5), Inches(0.8), Inches(0.8)),
                 '<a:solidFill><a:srgbClr val="808080"/></a:solidFill>')
    master_shape(master, 91, "Band", "rect", (0, Inches(7.2), Inches(10), Inches(0.3)),
                 '<a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="1F4E79"/></a:gs>'
                 '<a:gs pos="100000"><a:srgbClr val="9DC3E6"/></a:gs></a:gsLst></a:gradFill>')

    # 1-3. Slides showing the master's decorations.
    for n in range(3):
        s = blank(prs)
        tb = s.shapes.add_textbox(Inches(1), Inches(1), Inches(6), Inches(1))
        tb.text_frame.text = f"Extras {n + 1}"
    prs.save(out)


if __name__ == "__main__":
    out = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
    out.mkdir(parents=True, exist_ok=True)
    deck16x9(out / "deck16x9.pptx")
    deck4x3(out / "deck4x3.pptx")
    extras(out / "extras.pptx")
