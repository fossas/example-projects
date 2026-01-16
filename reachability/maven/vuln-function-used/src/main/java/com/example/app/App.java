package com.example.app;

import java.util.Map;
import java.net.URI;
import java.net.URL;
import com.example.app.utils.ContextReader;

// org.dom4j (CVE-2020-10683)
// ---------------------------
// dom4j before 2.0.3 and 2.1.x before 2.1.3 allows 
// external DTDs and External Entities by default, which might enable XXE attacks
import org.dom4j.Document;
import org.dom4j.DocumentException;
import org.dom4j.io.SAXReader;

public class App 
{
    public static void main(String[] args) throws Exception {
        URL url = new URI(args[0]).toURL();
        System.out.println(parse(url));
        System.out.println(ContextReader.parseWithCtx(url));
    }

    public static Document parse(URL url) throws DocumentException {
        SAXReader reader = new SAXReader();
        Document document = reader.read(url);
        return document;
    }
}
