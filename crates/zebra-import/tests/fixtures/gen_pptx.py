#!/usr/bin/env python3
"""Make the PowerPoint fixtures the import tests read (tests/pptx.rs).

    python3 gen_pptx.py OUT_DIR

Needs python-pptx (1.0.2), Pillow, and ffmpeg (with libx264 and aac) for
the tiny playable video and music. Writes deck16x9.pptx, deck4x3.pptx and
extras.pptx to OUT_DIR; copy them next to this script. Run it from a scratch directory,
not from here, so nothing else lands in the fixtures folder.
"""
import copy
import io
import struct
import subprocess
import sys
import tempfile
import wave
import zipfile
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
from pptx.opc.constants import RELATIONSHIP_TYPE as RT
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


def strike(s):
    """Strike-through, which python-pptx can't set: on runs, and from a
    text box's list style (a run can turn it off)."""
    tb = s.shapes.add_textbox(Inches(1), Inches(2.5), Inches(8), Inches(1))
    p = tb.text_frame.paragraphs[0]
    for text, how in [("plain ", None), ("single", "sngStrike"), (" and ", None), ("double", "dblStrike")]:
        r = p.add_run()
        r.text = text
        if how:
            r.font._rPr.set("strike", how)
    tb = s.shapes.add_textbox(Inches(1), Inches(4), Inches(8), Inches(1))
    lst = tb.text_frame._txBody.find(qn("a:lstStyle"))
    lvl = etree.SubElement(lst, qn("a:lvl1pPr"))
    etree.SubElement(lvl, qn("a:defRPr")).set("strike", "sngStrike")
    p = tb.text_frame.paragraphs[0]
    for text, how in [("struck ", None), ("not this", "noStrike")]:
        r = p.add_run()
        r.text = text
        if how:
            r.font._rPr.set("strike", how)


def ffmpeg(args, ext):
    """A tiny media file made by ffmpeg, as bytes."""
    with tempfile.TemporaryDirectory() as d:
        path = Path(d) / f"out.{ext}"
        subprocess.run(["ffmpeg", "-loglevel", "error", "-y", *args, "-map_metadata", "-1", "-fflags", "+bitexact", str(path)], check=True)
        return path.read_bytes()


def wav(secs):
    out = io.BytesIO()
    with wave.open(out, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(1)
        w.setframerate(8000)
        w.writeframes(bytes([128]) * int(8000 * secs))
    return out.getvalue()


def media_timing(s, xml):
    """Replace the slide's p:timing (python-pptx's says only "on click")."""
    old = s._element.find(qn("p:timing"))
    if old is not None:
        s._element.remove(old)
    s._element.append(etree.fromstring(f'<p:timing xmlns:p="{P}">{xml}</p:timing>'))


def as_audio(pic):
    """python-pptx adds videos only: make one a sound, as PowerPoint saves
    one (a:audioFile; p:audio in the timing; the relationship and file
    name are fixed by `repack` after saving)."""
    nv = pic._element.find(qn("p:nvPicPr")).find(qn("p:nvPr"))
    nv.find(qn("a:videoFile")).tag = qn("a:audioFile")


def repack(path, swaps):
    """Raw zip edits python-pptx can't make. `swaps` maps a part's name to
    (new name, new bytes or None, content type): its relationships follow
    it, and a sound's a:audioFile relationship gets the audio type."""
    with zipfile.ZipFile(path) as z:
        parts = {i.filename: z.read(i.filename) for i in z.infolist()}
    base = lambda n: n.rsplit("/", 1)[1]
    out = {}
    for name, data in parts.items():
        if name.endswith(".rels"):
            root = etree.fromstring(data)
            for rel in root:
                target = rel.get("Target", "")
                for old, (new, _, ct) in swaps.items():
                    if target.endswith("/" + base(old)):
                        rel.set("Target", target[: -len(base(old))] + base(new))
                        if ct.startswith("audio/") and rel.get("Type") == RT.VIDEO:
                            rel.set("Type", RT.AUDIO)
            data = etree.tostring(root, xml_declaration=True, encoding="UTF-8", standalone=True)
        elif name == "[Content_Types].xml":
            root = etree.fromstring(data)
            ns = "http://schemas.openxmlformats.org/package/2006/content-types"
            for o in root.findall(f"{{{ns}}}Override"):
                if o.get("PartName", "").lstrip("/") in swaps:
                    root.remove(o)
            have = {d.get("Extension") for d in root.findall(f"{{{ns}}}Default")}
            for new, _, ct in swaps.values():
                ext = new.rsplit(".", 1)[1]
                if ext not in have:
                    have.add(ext)
                    d = etree.SubElement(root, f"{{{ns}}}Default")
                    d.set("Extension", ext)
                    d.set("ContentType", ct)
            data = etree.tostring(root, xml_declaration=True, encoding="UTF-8", standalone=True)
        new, swapped, _ = swaps.get(name, (name, None, ""))
        out[new] = data if swapped is None else swapped
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        for name, data in out.items():
            z.writestr(name, data)


def sounds(path):
    """The sounds python-pptx saved as ppt/media/mediaN.vid."""
    with zipfile.ZipFile(path) as z:
        out = {}
        for n in z.namelist():
            if n.startswith("ppt/media/") and n.endswith(".vid"):
                wav = z.read(n)[:4] == b"RIFF"
                out[n] = (n[:-3] + ("wav" if wav else "m4a"), None, "audio/wav" if wav else "audio/mp4")
        return out


def media(prs):
    """Videos and sounds: one that plays on a Mac and starts by itself,
    looping and trimmed; one that won't play (WMV); a sound on its slide
    and music across the slides."""
    s = blank(prs)
    mp4 = ffmpeg(["-f", "lavfi", "-i", "color=c=0x2878c8:s=32x18:d=1:r=10", "-c:v", "libx264", "-pix_fmt", "yuv420p",
                  "-preset", "veryfast", "-movflags", "+faststart"], "mp4")
    v = s.shapes.add_movie(io.BytesIO(mp4), Inches(1), Inches(1), Inches(4), Inches(2.25),
                           poster_frame_image=png(32, 18, (40, 120, 200)), mime_type="video/mp4")
    v._element.find(qn("p:nvPicPr")).find(qn("p:cNvPr")).set("descr", "Blue")
    p14 = v._element.find(".//{http://schemas.microsoft.com/office/powerpoint/2010/main}media")
    etree.SubElement(p14, "{http://schemas.microsoft.com/office/powerpoint/2010/main}trim").set("st", "250")
    # An ASF header: Windows Media, which a Mac won't play.
    wmv = bytes.fromhex("3026b2758e66cf11a6d900aa0062ce6c") + bytes(32)
    s.shapes.add_movie(io.BytesIO(wmv), Inches(6), Inches(1), Inches(3), Inches(2),
                       poster_frame_image=png(30, 20, (200, 40, 40)), mime_type="video/x-ms-wmv")
    media_timing(s, f'''<p:tnLst><p:par><p:cTn id="1" dur="indefinite" restart="never" nodeType="tmRoot"><p:childTnLst>
<p:seq concurrent="1" nextAc="seek"><p:cTn id="2" dur="indefinite" nodeType="mainSeq"><p:childTnLst>
<p:par><p:cTn id="3" fill="hold"><p:stCondLst><p:cond delay="indefinite"/><p:cond evt="onBegin" delay="0"><p:tn val="2"/></p:cond></p:stCondLst><p:childTnLst>
<p:par><p:cTn id="4" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst>
<p:par><p:cTn id="5" presetID="1" presetClass="mediacall" presetSubtype="0" fill="hold" nodeType="afterEffect"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst>
<p:cmd type="call" cmd="playFrom(0.0)"><p:cBhvr><p:cTn id="6" dur="1000" fill="hold"/><p:tgtEl><p:spTgt spid="{v.shape_id}"/></p:tgtEl></p:cBhvr></p:cmd>
</p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par>
</p:childTnLst></p:cTn><p:prevCondLst><p:cond evt="onPrev" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:prevCondLst><p:nextCondLst><p:cond evt="onNext" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:nextCondLst></p:seq>
<p:video><p:cMediaNode vol="80000" mute="1"><p:cTn id="7" repeatCount="indefinite" fill="hold" display="0"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst></p:cTn><p:tgtEl><p:spTgt spid="{v.shape_id}"/></p:tgtEl></p:cMediaNode></p:video>
</p:childTnLst></p:cTn></p:par></p:tnLst>''')

    s = blank(prs)
    voice = s.shapes.add_movie(io.BytesIO(wav(0.2)), Inches(1), Inches(1), Inches(0.5), Inches(0.5),
                               poster_frame_image=png(16, 16, (90, 90, 90)), mime_type="audio/wav")
    as_audio(voice)
    m4a = ffmpeg(["-f", "lavfi", "-i", "sine=frequency=440:duration=0.5", "-c:a", "aac", "-b:a", "24k"], "m4a")
    music = s.shapes.add_movie(io.BytesIO(m4a), Inches(2), Inches(1), Inches(0.5), Inches(0.5),
                               poster_frame_image=png(16, 16, (90, 90, 90)), mime_type="audio/mp4")
    music.name = "Theme tune.m4a"
    as_audio(music)
    media_timing(s, f'''<p:tnLst><p:par><p:cTn id="1" dur="indefinite" restart="never" nodeType="tmRoot"><p:childTnLst>
<p:audio><p:cMediaNode vol="70000"><p:cTn id="2" fill="hold" display="0"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst></p:cTn><p:tgtEl><p:spTgt spid="{voice.shape_id}"/></p:tgtEl></p:cMediaNode></p:audio>
<p:audio><p:cMediaNode vol="40000" numSld="999" showWhenStopped="0"><p:cTn id="3" repeatCount="indefinite" fill="hold" display="0"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst></p:cTn><p:tgtEl><p:spTgt spid="{music.shape_id}"/></p:tgtEl></p:cMediaNode></p:audio>
</p:childTnLst></p:cTn></p:par></p:tnLst>''')


def emf():
    """A small EMF drawn by hand: a filled rectangle and an ellipse."""
    def rec(kind, body):
        return struct.pack("<II", kind, 8 + len(body)) + body

    def rectl(l, t, r, b):
        return struct.pack("<iiii", l, t, r, b)

    records = [
        rec(39, struct.pack("<IIII", 1, 0, 0x0040C0E0, 0)),  # CREATEBRUSHINDIRECT 1: solid, BGR e0c040
        rec(37, struct.pack("<I", 1)),  # SELECTOBJECT
        rec(38, struct.pack("<IIiiI", 2, 0, 4, 0, 0x00402010)),  # CREATEPEN 2: solid, 4 wide
        rec(37, struct.pack("<I", 2)),
        rec(43, rectl(10, 10, 190, 90)),  # RECTANGLE
        rec(42, rectl(60, 20, 140, 80)),  # ELLIPSE
        rec(14, struct.pack("<III", 0, 16, 20)),  # EOF
    ]
    body = b"".join(records)
    size = 88 + len(body)
    header = struct.pack("<II", 1, 88) + rectl(0, 0, 200, 100) + rectl(0, 0, 6250, 3125)
    header += struct.pack("<IIIIHHIIIiiii", 0x464D4520, 0x10000, size, len(records) + 1, 3, 0, 0, 0, 0, 1024, 768, 320, 240)
    assert len(header) == 88
    return header + body


def wmf():
    """A small placeable WMF drawn by hand: a filled rectangle."""
    def rec(func, params):
        return struct.pack("<IH", 3 + len(params) // 2, func) + params

    records = [
        rec(0x020B, struct.pack("<hh", 0, 0)),  # SETWINDOWORG (y, x)
        rec(0x020C, struct.pack("<hh", 100, 200)),  # SETWINDOWEXT (y, x)
        rec(0x02FC, struct.pack("<HIH", 0, 0x0030A040, 0)),  # CREATEBRUSHINDIRECT
        rec(0x012D, struct.pack("<H", 0)),  # SELECTOBJECT
        rec(0x041B, struct.pack("<hhhh", 90, 190, 10, 10)),  # RECTANGLE (bottom, right, top, left)
        rec(0x0000, b""),  # EOF
    ]
    body = b"".join(records)
    words = (18 + len(body)) // 2
    biggest = max(len(r) for r in records) // 2
    meta = struct.pack("<HHHIHIH", 1, 9, 0x0300, words, 1, biggest, 0)
    place = struct.pack("<IHhhhhHI", 0x9AC6CDD7, 0, 0, 0, 200, 100, 96, 0)
    check = 0
    for (w,) in struct.iter_unpack("<H", place):
        check ^= w
    return place + struct.pack("<H", check) + meta + body


def metafiles(prs):
    """EMF and WMF pictures, and one that can't be drawn. python-pptx
    takes them as PNGs; `repack` puts the metafiles in after saving."""
    s = blank(prs)
    swaps = {}
    broken = struct.pack("<II", 1, 88) + bytes(32) + struct.pack("<I", 0x464D4520) + bytes(44)
    for i, (data, ext, ct) in enumerate([(emf(), "emf", "image/x-emf"), (wmf(), "wmf", "image/x-wmf"), (broken, "emf", "image/x-emf")]):
        pic = s.shapes.add_picture(png(20, 10 + i, (i * 60, 0, 0)), Inches(0.5 + 3 * i), Inches(1), Inches(2.8), Inches(1.4))
        part = s.part.related_part(pic._element.blip_rId)
        name = str(part.partname).lstrip("/")
        swaps[name] = (name.rsplit(".", 1)[0] + "." + ext, data, ct)
    return swaps


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
        if n == 0:
            strike(s)
    # 4-5. Videos and sounds.
    media(prs)
    # 6. EMF and WMF.
    swaps = metafiles(prs)
    prs.save(out)
    repack(out, {**sounds(out), **swaps})


if __name__ == "__main__":
    out = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
    out.mkdir(parents=True, exist_ok=True)
    deck16x9(out / "deck16x9.pptx")
    deck4x3(out / "deck4x3.pptx")
    extras(out / "extras.pptx")
